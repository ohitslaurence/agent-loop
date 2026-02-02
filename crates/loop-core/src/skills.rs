//! Core skill types and frontmatter parsing utilities.
//!
//! Implements the Agent Skills SKILL.md format for orchestrator skill support.
//! See: https://agentskills.io/specification

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// Skill location indicating where the skill was discovered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillLocation {
    /// Skill from a project-local directory (e.g., `.agent/skills`).
    Project,
    /// Skill from a global directory (e.g., `~/.agent/skills`).
    Global,
}

impl SkillLocation {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Project => "project",
            Self::Global => "global",
        }
    }
}

/// Metadata extracted from a SKILL.md frontmatter.
///
/// Per Agent Skills spec:
/// - `name`: 1-64 chars, lowercase letters/numbers/hyphens, no leading/trailing/consecutive hyphens
/// - `description`: 1-1024 chars, describes what the skill does and when to use it
/// - `license`, `compatibility`, `metadata`, `allowed_tools`: optional
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillMetadata {
    /// Skill name (1-64 chars, lowercase alphanumeric + hyphens).
    pub name: String,
    /// Description of what the skill does and when to use it (1-1024 chars).
    pub description: String,
    /// Optional license name or reference.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
    /// Optional compatibility notes (max 500 chars).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compatibility: Option<String>,
    /// Optional arbitrary key-value metadata.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub metadata: HashMap<String, String>,
    /// Optional space-delimited list of pre-approved tools.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_tools: Vec<String>,
    /// Absolute path to the skill directory.
    pub path: PathBuf,
    /// Where the skill was discovered.
    pub location: SkillLocation,
}

/// Error type for skill parsing and validation.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SkillError {
    #[error("missing YAML frontmatter")]
    MissingFrontmatter,
    #[error("invalid YAML frontmatter: {0}")]
    InvalidYaml(String),
    #[error("missing required field: {0}")]
    MissingField(&'static str),
    #[error("invalid name: {0}")]
    InvalidName(String),
    #[error("invalid description: {0}")]
    InvalidDescription(String),
    #[error("invalid compatibility: {0}")]
    InvalidCompatibility(String),
}

/// Raw frontmatter as parsed from YAML.
#[derive(Debug, Deserialize)]
struct RawFrontmatter {
    name: Option<String>,
    description: Option<String>,
    license: Option<String>,
    compatibility: Option<String>,
    metadata: Option<HashMap<String, String>>,
    #[serde(rename = "allowed-tools")]
    allowed_tools: Option<String>,
}

/// Validates a skill name according to Agent Skills spec.
///
/// Rules:
/// - 1-64 characters
/// - Lowercase letters, numbers, and hyphens only
/// - Must not start or end with hyphen
/// - Must not contain consecutive hyphens
pub fn validate_name(name: &str) -> Result<(), SkillError> {
    if name.is_empty() {
        return Err(SkillError::InvalidName("name cannot be empty".to_string()));
    }
    if name.len() > 64 {
        return Err(SkillError::InvalidName(format!(
            "name exceeds 64 characters (got {})",
            name.len()
        )));
    }
    if name.starts_with('-') {
        return Err(SkillError::InvalidName(
            "name cannot start with hyphen".to_string(),
        ));
    }
    if name.ends_with('-') {
        return Err(SkillError::InvalidName(
            "name cannot end with hyphen".to_string(),
        ));
    }
    if name.contains("--") {
        return Err(SkillError::InvalidName(
            "name cannot contain consecutive hyphens".to_string(),
        ));
    }
    for c in name.chars() {
        if !c.is_ascii_lowercase() && !c.is_ascii_digit() && c != '-' {
            return Err(SkillError::InvalidName(format!(
                "invalid character '{}': only lowercase letters, numbers, and hyphens allowed",
                c
            )));
        }
    }
    Ok(())
}

/// Validates a skill description according to Agent Skills spec.
///
/// Rules:
/// - 1-1024 characters
/// - Non-empty
pub fn validate_description(description: &str) -> Result<(), SkillError> {
    if description.is_empty() {
        return Err(SkillError::InvalidDescription(
            "description cannot be empty".to_string(),
        ));
    }
    if description.len() > 1024 {
        return Err(SkillError::InvalidDescription(format!(
            "description exceeds 1024 characters (got {})",
            description.len()
        )));
    }
    Ok(())
}

/// Validates a skill compatibility field according to Agent Skills spec.
///
/// Rules:
/// - 1-500 characters if provided
fn validate_compatibility(compatibility: &str) -> Result<(), SkillError> {
    if compatibility.is_empty() {
        return Err(SkillError::InvalidCompatibility(
            "compatibility cannot be empty if provided".to_string(),
        ));
    }
    if compatibility.len() > 500 {
        return Err(SkillError::InvalidCompatibility(format!(
            "compatibility exceeds 500 characters (got {})",
            compatibility.len()
        )));
    }
    Ok(())
}

/// Extracts YAML frontmatter from SKILL.md content.
///
/// Frontmatter must be delimited by `---` lines at the start of the file.
fn extract_frontmatter(content: &str) -> Result<&str, SkillError> {
    let trimmed = content.trim_start();
    if !trimmed.starts_with("---") {
        return Err(SkillError::MissingFrontmatter);
    }

    // Skip opening delimiter
    let after_open = &trimmed[3..];
    let after_newline = after_open
        .strip_prefix('\n')
        .or_else(|| after_open.strip_prefix("\r\n"))
        .unwrap_or(after_open);

    // Find closing delimiter
    let close_pos = after_newline
        .find("\n---")
        .or_else(|| after_newline.find("\r\n---"));

    match close_pos {
        Some(pos) => Ok(&after_newline[..pos]),
        None => Err(SkillError::MissingFrontmatter),
    }
}

/// Parses SKILL.md content and extracts validated metadata.
///
/// Returns the metadata or an error if parsing/validation fails.
pub fn parse_skill_md(
    content: &str,
    path: PathBuf,
    location: SkillLocation,
) -> Result<SkillMetadata, SkillError> {
    let frontmatter_str = extract_frontmatter(content)?;

    let raw: RawFrontmatter = serde_yaml::from_str(frontmatter_str)
        .map_err(|e| SkillError::InvalidYaml(e.to_string()))?;

    let name = raw.name.ok_or(SkillError::MissingField("name"))?;
    validate_name(&name)?;

    let description = raw
        .description
        .ok_or(SkillError::MissingField("description"))?;
    validate_description(&description)?;

    if let Some(ref compat) = raw.compatibility {
        validate_compatibility(compat)?;
    }

    // Parse allowed-tools from space-delimited string
    let allowed_tools = raw
        .allowed_tools
        .map(|s| s.split_whitespace().map(String::from).collect())
        .unwrap_or_default();

    Ok(SkillMetadata {
        name,
        description,
        license: raw.license,
        compatibility: raw.compatibility,
        metadata: raw.metadata.unwrap_or_default(),
        allowed_tools,
        path,
        location,
    })
}

/// Extracts the body content after the YAML frontmatter.
///
/// Returns the markdown body or an empty string if no body exists.
pub fn extract_body(content: &str) -> Result<&str, SkillError> {
    let trimmed = content.trim_start();
    if !trimmed.starts_with("---") {
        return Err(SkillError::MissingFrontmatter);
    }

    // Skip opening delimiter
    let after_open = &trimmed[3..];
    let after_newline = after_open
        .strip_prefix('\n')
        .or_else(|| after_open.strip_prefix("\r\n"))
        .unwrap_or(after_open);

    // Find closing delimiter
    if let Some(pos) = after_newline.find("\n---") {
        let after_close = &after_newline[pos + 4..];
        // Skip newline after closing ---
        let body = after_close
            .strip_prefix('\n')
            .or_else(|| after_close.strip_prefix("\r\n"))
            .unwrap_or(after_close);
        Ok(body.trim())
    } else if let Some(pos) = after_newline.find("\r\n---") {
        let after_close = &after_newline[pos + 5..];
        let body = after_close
            .strip_prefix('\n')
            .or_else(|| after_close.strip_prefix("\r\n"))
            .unwrap_or(after_close);
        Ok(body.trim())
    } else {
        Err(SkillError::MissingFrontmatter)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_name_accepts_valid_names() {
        assert!(validate_name("pdf").is_ok());
        assert!(validate_name("pdf-processing").is_ok());
        assert!(validate_name("code-review").is_ok());
        assert!(validate_name("data-analysis").is_ok());
        assert!(validate_name("a1b2c3").is_ok());
        assert!(validate_name("skill123").is_ok());
        // Max length (64 chars)
        assert!(validate_name(&"a".repeat(64)).is_ok());
    }

    #[test]
    fn validate_name_rejects_empty() {
        let err = validate_name("").unwrap_err();
        assert!(matches!(err, SkillError::InvalidName(_)));
    }

    #[test]
    fn validate_name_rejects_too_long() {
        let err = validate_name(&"a".repeat(65)).unwrap_err();
        assert!(matches!(err, SkillError::InvalidName(_)));
    }

    #[test]
    fn validate_name_rejects_uppercase() {
        let err = validate_name("PDF-Processing").unwrap_err();
        assert!(matches!(err, SkillError::InvalidName(_)));
    }

    #[test]
    fn validate_name_rejects_leading_hyphen() {
        let err = validate_name("-pdf").unwrap_err();
        assert!(matches!(err, SkillError::InvalidName(_)));
    }

    #[test]
    fn validate_name_rejects_trailing_hyphen() {
        let err = validate_name("pdf-").unwrap_err();
        assert!(matches!(err, SkillError::InvalidName(_)));
    }

    #[test]
    fn validate_name_rejects_consecutive_hyphens() {
        let err = validate_name("pdf--processing").unwrap_err();
        assert!(matches!(err, SkillError::InvalidName(_)));
    }

    #[test]
    fn validate_name_rejects_underscores() {
        let err = validate_name("pdf_processing").unwrap_err();
        assert!(matches!(err, SkillError::InvalidName(_)));
    }

    #[test]
    fn validate_name_rejects_spaces() {
        let err = validate_name("pdf processing").unwrap_err();
        assert!(matches!(err, SkillError::InvalidName(_)));
    }

    #[test]
    fn validate_description_accepts_valid() {
        assert!(validate_description("A simple skill.").is_ok());
        assert!(validate_description(&"x".repeat(1024)).is_ok());
    }

    #[test]
    fn validate_description_rejects_empty() {
        let err = validate_description("").unwrap_err();
        assert!(matches!(err, SkillError::InvalidDescription(_)));
    }

    #[test]
    fn validate_description_rejects_too_long() {
        let err = validate_description(&"x".repeat(1025)).unwrap_err();
        assert!(matches!(err, SkillError::InvalidDescription(_)));
    }

    #[test]
    fn parse_skill_md_basic() {
        let content = r#"---
name: pdf-processing
description: Extract text and tables from PDF files.
---

# PDF Processing

Instructions here.
"#;
        let meta = parse_skill_md(
            content,
            PathBuf::from("/skills/pdf"),
            SkillLocation::Project,
        )
        .expect("should parse");
        assert_eq!(meta.name, "pdf-processing");
        assert_eq!(meta.description, "Extract text and tables from PDF files.");
        assert!(meta.license.is_none());
        assert!(meta.compatibility.is_none());
        assert!(meta.metadata.is_empty());
        assert!(meta.allowed_tools.is_empty());
        assert_eq!(meta.path, PathBuf::from("/skills/pdf"));
        assert_eq!(meta.location, SkillLocation::Project);
    }

    #[test]
    fn parse_skill_md_with_optional_fields() {
        let content = r#"---
name: code-review
description: Review code for best practices and potential issues.
license: Apache-2.0
compatibility: Requires git
metadata:
  author: example-org
  version: "1.0"
allowed-tools: Bash(git:*) Read
---

Body content.
"#;
        let meta = parse_skill_md(
            content,
            PathBuf::from("/skills/review"),
            SkillLocation::Global,
        )
        .expect("should parse");
        assert_eq!(meta.name, "code-review");
        assert_eq!(meta.license, Some("Apache-2.0".to_string()));
        assert_eq!(meta.compatibility, Some("Requires git".to_string()));
        assert_eq!(
            meta.metadata.get("author"),
            Some(&"example-org".to_string())
        );
        assert_eq!(meta.metadata.get("version"), Some(&"1.0".to_string()));
        assert_eq!(meta.allowed_tools, vec!["Bash(git:*)", "Read"]);
        assert_eq!(meta.location, SkillLocation::Global);
    }

    #[test]
    fn parse_skill_md_missing_frontmatter() {
        let content = "# No frontmatter\n\nJust markdown.";
        let err = parse_skill_md(
            content,
            PathBuf::from("/skills/bad"),
            SkillLocation::Project,
        )
        .unwrap_err();
        assert!(matches!(err, SkillError::MissingFrontmatter));
    }

    #[test]
    fn parse_skill_md_missing_closing_delimiter() {
        let content = r#"---
name: bad
description: No closing
"#;
        let err = parse_skill_md(
            content,
            PathBuf::from("/skills/bad"),
            SkillLocation::Project,
        )
        .unwrap_err();
        assert!(matches!(err, SkillError::MissingFrontmatter));
    }

    #[test]
    fn parse_skill_md_missing_name() {
        let content = r#"---
description: Has description but no name.
---
"#;
        let err = parse_skill_md(
            content,
            PathBuf::from("/skills/bad"),
            SkillLocation::Project,
        )
        .unwrap_err();
        assert!(matches!(err, SkillError::MissingField("name")));
    }

    #[test]
    fn parse_skill_md_missing_description() {
        let content = r#"---
name: has-name
---
"#;
        let err = parse_skill_md(
            content,
            PathBuf::from("/skills/bad"),
            SkillLocation::Project,
        )
        .unwrap_err();
        assert!(matches!(err, SkillError::MissingField("description")));
    }

    #[test]
    fn parse_skill_md_invalid_name() {
        let content = r#"---
name: INVALID-NAME
description: Valid description.
---
"#;
        let err = parse_skill_md(
            content,
            PathBuf::from("/skills/bad"),
            SkillLocation::Project,
        )
        .unwrap_err();
        assert!(matches!(err, SkillError::InvalidName(_)));
    }

    #[test]
    fn parse_skill_md_empty_description() {
        let content = r#"---
name: valid-name
description: ""
---
"#;
        let err = parse_skill_md(
            content,
            PathBuf::from("/skills/bad"),
            SkillLocation::Project,
        )
        .unwrap_err();
        assert!(matches!(err, SkillError::InvalidDescription(_)));
    }

    #[test]
    fn parse_skill_md_invalid_compatibility() {
        let too_long = "x".repeat(501);
        let content = format!(
            r#"---
name: valid-name
description: Valid description.
compatibility: {}
---
"#,
            too_long
        );
        let err = parse_skill_md(
            &content,
            PathBuf::from("/skills/bad"),
            SkillLocation::Project,
        )
        .unwrap_err();
        assert!(matches!(err, SkillError::InvalidCompatibility(_)));
    }

    #[test]
    fn extract_body_returns_content() {
        let content = r#"---
name: test
description: Test skill.
---

# Instructions

Do this thing.
"#;
        let body = extract_body(content).expect("should extract body");
        assert!(body.contains("# Instructions"));
        assert!(body.contains("Do this thing."));
    }

    #[test]
    fn extract_body_empty() {
        let content = r#"---
name: test
description: Test skill.
---
"#;
        let body = extract_body(content).expect("should extract body");
        assert!(body.is_empty());
    }

    #[test]
    fn skill_location_serializes() {
        assert_eq!(
            serde_json::to_string(&SkillLocation::Project).unwrap(),
            "\"project\""
        );
        assert_eq!(
            serde_json::to_string(&SkillLocation::Global).unwrap(),
            "\"global\""
        );
    }

    #[test]
    fn skill_metadata_serializes() {
        let meta = SkillMetadata {
            name: "test".to_string(),
            description: "A test skill.".to_string(),
            license: None,
            compatibility: None,
            metadata: HashMap::new(),
            allowed_tools: vec![],
            path: PathBuf::from("/skills/test"),
            location: SkillLocation::Project,
        };
        let json = serde_json::to_string(&meta).expect("should serialize");
        assert!(json.contains("\"name\":\"test\""));
        assert!(json.contains("\"location\":\"project\""));
    }
}
