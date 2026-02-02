//! Skill rendering for prompt integration.
//!
//! Implements spec Section 4.2 and 5.1: render available skills as an XML block
//! in the OpenSkills format for prompt inclusion.

use loop_core::skills::SkillMetadata;
use std::fs;
use std::io;
use std::path::Path;

/// Renders available skills as an XML block in OpenSkills format.
///
/// Per spec Section 4.2 and 5.1: matches the OpenSkills XML block format
/// with `<available_skills>` container and individual `<skill>` entries.
///
/// The format includes:
/// - `<skills_system priority="1">` wrapper
/// - `<usage>` instructions for how to use skills
/// - `<available_skills>` with individual skill entries
///
/// Each skill entry contains:
/// - `<name>`: skill identifier
/// - `<description>`: what the skill does
/// - `<location>`: project or global
pub fn render_available_skills(skills: &[SkillMetadata]) -> String {
    if skills.is_empty() {
        return String::new();
    }

    let skill_tags: Vec<String> = skills
        .iter()
        .map(|s| {
            format!(
                "<skill>\n<name>{}</name>\n<description>{}</description>\n<location>{}</location>\n</skill>",
                escape_xml(&s.name),
                escape_xml(&s.description),
                s.location.as_str()
            )
        })
        .collect();

    format!(
        r#"<skills_system priority="1">

## Available Skills

<!-- SKILLS_TABLE_START -->
<usage>
When users ask you to perform tasks, check if any of the available skills below can help complete the task more effectively. Skills provide specialized capabilities and domain knowledge.

How to use skills:
- Skills listed below have been automatically loaded based on the current task
- The skill instructions are included after this section with detailed guidance
- Use the skill's base directory for resolving bundled resources (references/, scripts/, assets/)

Usage notes:
- Only use skills that are loaded in your context
- Each skill provides domain-specific instructions
</usage>

<available_skills>

{}

</available_skills>
<!-- SKILLS_TABLE_END -->

</skills_system>"#,
        skill_tags.join("\n\n")
    )
}

/// Escapes special XML characters in text.
fn escape_xml(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// Result of loading a skill body.
#[derive(Debug, Clone)]
pub struct LoadedSkill {
    /// The formatted skill content in OpenSkills `read` format.
    pub content: String,
    /// Whether the content was truncated.
    pub truncated: bool,
    /// Original size before truncation (if truncated).
    pub original_size: Option<usize>,
}

/// Truncation event info for async emission.
///
/// Per open-skills-orchestration.md Section 4.3: SKILLS_TRUNCATED event.
#[derive(Debug, Clone)]
pub struct TruncationEvent {
    /// The skill name that was truncated.
    pub name: String,
    /// The maximum character limit that was applied.
    pub max_chars: usize,
}

/// Load failure event info for async emission.
///
/// Per open-skills-orchestration.md Section 4.3: SKILLS_LOAD_FAILED event.
#[derive(Debug, Clone)]
pub struct LoadFailureEvent {
    /// The skill name that failed to load.
    pub name: String,
    /// The error message describing the failure.
    pub error: String,
}

/// Loads a skill's SKILL.md content in OpenSkills `read` output format.
///
/// Per spec Section 4.2 and 5.1: mirrors the OpenSkills `read` output format
/// with `Reading:`, `Base directory:`, content, and `Skill read:` lines.
///
/// The output format:
/// ```text
/// Reading: <skill-name>
/// Base directory: <absolute-path-to-skill-directory>
///
/// <SKILL.md content>
///
/// Skill read: <skill-name>
/// ```
///
/// # Arguments
/// * `skill` - The skill metadata (must have a valid path).
/// * `include_references` - Whether to include contents from `references/` directory.
/// * `max_chars` - Maximum characters to include from the SKILL.md body.
///
/// # Returns
/// A `LoadedSkill` with the formatted content and truncation info.
///
/// # Errors
/// Returns an IO error if the SKILL.md file cannot be read.
pub fn load_skill_body(
    skill: &SkillMetadata,
    include_references: bool,
    max_chars: usize,
) -> io::Result<LoadedSkill> {
    let skill_file = skill.path.join("SKILL.md");
    let mut content = fs::read_to_string(&skill_file)?;
    let original_size = content.len();
    let mut truncated = false;

    // Optionally include references/ directory content.
    if include_references {
        let references_dir = skill.path.join("references");
        if references_dir.is_dir() {
            if let Ok(references_content) = load_references_dir(&references_dir) {
                content.push_str("\n\n---\n\n## References\n\n");
                content.push_str(&references_content);
            }
        }
    }

    // Truncate if exceeds max_chars.
    if content.len() > max_chars {
        // Truncate at a safe boundary (don't split multi-byte chars).
        let truncate_at = content
            .char_indices()
            .take_while(|(i, _)| *i < max_chars)
            .last()
            .map(|(i, c)| i + c.len_utf8())
            .unwrap_or(0);

        content.truncate(truncate_at);
        content.push_str("\n\n[Content truncated...]");
        truncated = true;
    }

    // Format in OpenSkills `read` output format.
    let base_dir = skill.path.display();
    let formatted = format!(
        "Reading: {}\nBase directory: {}\n\n{}\n\nSkill read: {}",
        skill.name, base_dir, content, skill.name
    );

    Ok(LoadedSkill {
        content: formatted,
        truncated,
        original_size: if truncated { Some(original_size) } else { None },
    })
}

/// Loads all markdown files from the references/ directory.
fn load_references_dir(references_dir: &Path) -> io::Result<String> {
    let mut content = String::new();
    let entries: Vec<_> = fs::read_dir(references_dir)?
        .filter_map(Result::ok)
        .filter(|e| e.path().extension().is_some_and(|ext| ext == "md"))
        .collect();

    for (i, entry) in entries.iter().enumerate() {
        let path = entry.path();
        let file_name = path.file_name().unwrap_or_default().to_string_lossy();
        let file_content = fs::read_to_string(&path)?;

        if i > 0 {
            content.push_str("\n\n---\n\n");
        }
        content.push_str(&format!("### {}\n\n{}", file_name, file_content));
    }

    Ok(content)
}

#[cfg(test)]
mod tests {
    use super::*;
    use loop_core::skills::SkillLocation;
    use std::collections::HashMap;
    use std::path::PathBuf;
    use tempfile::TempDir;

    fn make_skill(name: &str, description: &str, location: SkillLocation) -> SkillMetadata {
        SkillMetadata {
            name: name.to_string(),
            description: description.to_string(),
            license: None,
            compatibility: None,
            metadata: HashMap::new(),
            allowed_tools: Vec::new(),
            path: PathBuf::from(format!("/skills/{}", name)),
            location,
        }
    }

    fn make_skill_with_path(name: &str, path: PathBuf) -> SkillMetadata {
        SkillMetadata {
            name: name.to_string(),
            description: "Test skill.".to_string(),
            license: None,
            compatibility: None,
            metadata: HashMap::new(),
            allowed_tools: Vec::new(),
            path,
            location: SkillLocation::Project,
        }
    }

    #[test]
    fn renders_empty_list() {
        let result = render_available_skills(&[]);
        assert!(result.is_empty());
    }

    #[test]
    fn renders_single_skill() {
        let skills = vec![make_skill(
            "pdf-processing",
            "Extract text from PDF files.",
            SkillLocation::Project,
        )];

        let result = render_available_skills(&skills);

        assert!(result.contains("<skills_system priority=\"1\">"));
        assert!(result.contains("<available_skills>"));
        assert!(result.contains("<name>pdf-processing</name>"));
        assert!(result.contains("<description>Extract text from PDF files.</description>"));
        assert!(result.contains("<location>project</location>"));
        assert!(result.contains("</skills_system>"));
    }

    #[test]
    fn renders_multiple_skills() {
        let skills = vec![
            make_skill(
                "pdf-processing",
                "Extract text from PDF files.",
                SkillLocation::Project,
            ),
            make_skill(
                "code-review",
                "Review code for best practices.",
                SkillLocation::Global,
            ),
        ];

        let result = render_available_skills(&skills);

        assert!(result.contains("<name>pdf-processing</name>"));
        assert!(result.contains("<name>code-review</name>"));
        assert!(result.contains("<location>project</location>"));
        assert!(result.contains("<location>global</location>"));
    }

    #[test]
    fn escapes_xml_characters() {
        let skills = vec![make_skill(
            "test-skill",
            "Handle <special> & \"quoted\" 'chars'.",
            SkillLocation::Project,
        )];

        let result = render_available_skills(&skills);

        assert!(result.contains("&lt;special&gt;"));
        assert!(result.contains("&amp;"));
        assert!(result.contains("&quot;quoted&quot;"));
        assert!(result.contains("&apos;chars&apos;"));
    }

    #[test]
    fn includes_usage_instructions() {
        let skills = vec![make_skill(
            "test-skill",
            "A test skill.",
            SkillLocation::Project,
        )];

        let result = render_available_skills(&skills);

        assert!(result.contains("<usage>"));
        assert!(result.contains("</usage>"));
        assert!(result.contains("Available Skills"));
        assert!(result.contains("<!-- SKILLS_TABLE_START -->"));
        assert!(result.contains("<!-- SKILLS_TABLE_END -->"));
    }

    #[test]
    fn result_is_valid_structure() {
        let skills = vec![make_skill(
            "test-skill",
            "A test skill.",
            SkillLocation::Project,
        )];

        let result = render_available_skills(&skills);

        // Check structure order
        let skills_system_start = result.find("<skills_system").unwrap();
        let available_skills_start = result.find("<available_skills>").unwrap();
        let skill_start = result.find("<skill>").unwrap();
        let skill_end = result.find("</skill>").unwrap();
        let available_skills_end = result.find("</available_skills>").unwrap();
        let skills_system_end = result.find("</skills_system>").unwrap();

        assert!(skills_system_start < available_skills_start);
        assert!(available_skills_start < skill_start);
        assert!(skill_start < skill_end);
        assert!(skill_end < available_skills_end);
        assert!(available_skills_end < skills_system_end);
    }

    #[test]
    fn load_skill_body_formats_correctly() {
        let temp_dir = TempDir::new().unwrap();
        let skill_dir = temp_dir.path().join("test-skill");
        fs::create_dir_all(&skill_dir).unwrap();

        let skill_content = r#"---
name: test-skill
description: A test skill.
---

# Test Skill

Instructions here.
"#;
        fs::write(skill_dir.join("SKILL.md"), skill_content).unwrap();

        let skill = make_skill_with_path("test-skill", skill_dir.clone());
        let result = load_skill_body(&skill, false, 20000).unwrap();

        assert!(result.content.starts_with("Reading: test-skill\n"));
        assert!(result
            .content
            .contains(&format!("Base directory: {}", skill_dir.display())));
        assert!(result.content.contains("# Test Skill"));
        assert!(result.content.contains("Instructions here."));
        assert!(result.content.ends_with("\n\nSkill read: test-skill"));
        assert!(!result.truncated);
        assert!(result.original_size.is_none());
    }

    #[test]
    fn load_skill_body_truncates_long_content() {
        let temp_dir = TempDir::new().unwrap();
        let skill_dir = temp_dir.path().join("long-skill");
        fs::create_dir_all(&skill_dir).unwrap();

        // Create content that exceeds max_chars.
        let long_content = format!(
            r#"---
name: long-skill
description: A skill with long content.
---

# Long Skill

{}
"#,
            "x".repeat(500)
        );
        fs::write(skill_dir.join("SKILL.md"), &long_content).unwrap();

        let skill = make_skill_with_path("long-skill", skill_dir.clone());
        let result = load_skill_body(&skill, false, 100).unwrap();

        assert!(result.truncated);
        assert!(result.original_size.is_some());
        assert!(result.original_size.unwrap() > 100);
        assert!(result.content.contains("[Content truncated...]"));
        // Still has proper header/footer format.
        assert!(result.content.starts_with("Reading: long-skill\n"));
        assert!(result.content.ends_with("\n\nSkill read: long-skill"));
    }

    #[test]
    fn load_skill_body_includes_references() {
        let temp_dir = TempDir::new().unwrap();
        let skill_dir = temp_dir.path().join("ref-skill");
        let refs_dir = skill_dir.join("references");
        fs::create_dir_all(&refs_dir).unwrap();

        let skill_content = r#"---
name: ref-skill
description: A skill with references.
---

# Ref Skill

See references for more info.
"#;
        fs::write(skill_dir.join("SKILL.md"), skill_content).unwrap();
        fs::write(refs_dir.join("guide.md"), "# Guide\n\nReference content.").unwrap();

        let skill = make_skill_with_path("ref-skill", skill_dir.clone());
        let result = load_skill_body(&skill, true, 20000).unwrap();

        assert!(result.content.contains("## References"));
        assert!(result.content.contains("### guide.md"));
        assert!(result.content.contains("Reference content."));
        assert!(!result.truncated);
    }

    #[test]
    fn load_skill_body_skips_references_when_disabled() {
        let temp_dir = TempDir::new().unwrap();
        let skill_dir = temp_dir.path().join("ref-skill-disabled");
        let refs_dir = skill_dir.join("references");
        fs::create_dir_all(&refs_dir).unwrap();

        let skill_content = r#"---
name: ref-skill
description: A skill with references.
---

# Ref Skill
"#;
        fs::write(skill_dir.join("SKILL.md"), skill_content).unwrap();
        fs::write(refs_dir.join("guide.md"), "# Guide").unwrap();

        let skill = make_skill_with_path("ref-skill", skill_dir.clone());
        let result = load_skill_body(&skill, false, 20000).unwrap();

        assert!(!result.content.contains("## References"));
        assert!(!result.content.contains("### guide.md"));
    }

    #[test]
    fn load_skill_body_handles_missing_file() {
        let temp_dir = TempDir::new().unwrap();
        let skill_dir = temp_dir.path().join("missing-skill");
        fs::create_dir_all(&skill_dir).unwrap();
        // Don't create SKILL.md.

        let skill = make_skill_with_path("missing-skill", skill_dir);
        let result = load_skill_body(&skill, false, 20000);

        assert!(result.is_err());
    }

    #[test]
    fn load_skill_body_handles_unicode() {
        let temp_dir = TempDir::new().unwrap();
        let skill_dir = temp_dir.path().join("unicode-skill");
        fs::create_dir_all(&skill_dir).unwrap();

        let skill_content = r#"---
name: unicode-skill
description: A skill with unicode.
---

# 日本語スキル

Instructions with 中文 and émojis 🎉.
"#;
        fs::write(skill_dir.join("SKILL.md"), skill_content).unwrap();

        let skill = make_skill_with_path("unicode-skill", skill_dir);
        // Truncate in the middle of unicode content.
        let result = load_skill_body(&skill, false, 80).unwrap();

        // Should not panic and should produce valid UTF-8.
        assert!(result.truncated);
        assert!(result.content.starts_with("Reading: unicode-skill\n"));
    }
}
