//! Skill rendering for prompt integration.
//!
//! Implements spec Section 4.2 and 5.1: render available skills as an XML block
//! in the OpenSkills format for prompt inclusion.

use loop_core::skills::SkillMetadata;

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

#[cfg(test)]
mod tests {
    use super::*;
    use loop_core::skills::SkillLocation;
    use std::collections::HashMap;
    use std::path::PathBuf;

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
}
