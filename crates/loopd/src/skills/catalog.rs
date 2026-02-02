//! Skill catalog: directory scanning and discovery.
//!
//! Implements spec Section 4.2 and 5.1: Discover skills from configured directories,
//! parse SKILL.md frontmatter, and return a list of available skills.

use loop_core::config::Config;
use loop_core::skills::{parse_skill_md, SkillError, SkillLocation, SkillMetadata};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use tracing::{debug, warn};

/// Result of skill discovery with potential errors.
#[derive(Debug)]
pub struct DiscoveryResult {
    /// Successfully parsed skills.
    pub skills: Vec<SkillMetadata>,
    /// Parse/load errors encountered.
    pub errors: Vec<DiscoveryError>,
}

/// Parse/load error details for a skill.
#[derive(Debug)]
pub struct DiscoveryError {
    /// Skill name (directory name fallback).
    pub name: String,
    /// Path to the SKILL.md file that failed.
    pub path: PathBuf,
    /// Underlying parse/load error.
    pub error: SkillError,
}

/// Discover all skills from configured directories.
///
/// Scans directories in priority order (per spec Section 5.1):
/// 1. Project-local directories (e.g., `.agent/skills`, `.claude/skills`)
/// 2. Global directories (e.g., `~/.agent/skills`, `~/.claude/skills`)
/// 3. Synced built-in skills directory
///
/// Per spec Section 5.2:
/// - Invalid SKILL.md frontmatter: skip skill, record error
/// - Duplicate skill names: prefer first match in configured search order
/// - Missing or empty description: skip skill (violates spec)
pub fn discover_skills(config: &Config, workspace_root: &Path) -> DiscoveryResult {
    let mut skills = Vec::new();
    let mut errors = Vec::new();
    let mut seen_names: HashSet<String> = HashSet::new();

    // Scan configured directories in priority order.
    for dir in &config.skills_dirs {
        let resolved = if dir.is_relative() {
            workspace_root.join(dir)
        } else {
            dir.clone()
        };

        // Determine location based on whether it's under workspace root.
        let location = if resolved.starts_with(workspace_root) {
            SkillLocation::Project
        } else {
            SkillLocation::Global
        };

        scan_directory(
            &resolved,
            location,
            &mut skills,
            &mut errors,
            &mut seen_names,
        );
    }

    // Also scan the synced built-in skills directory.
    if config.skills_sync_on_start {
        scan_directory(
            &config.skills_sync_dir,
            SkillLocation::Global,
            &mut skills,
            &mut errors,
            &mut seen_names,
        );
    }

    debug!(
        count = skills.len(),
        errors = errors.len(),
        "discovered skills"
    );

    DiscoveryResult { skills, errors }
}

/// Scan a single directory for skills.
fn scan_directory(
    dir: &Path,
    location: SkillLocation,
    skills: &mut Vec<SkillMetadata>,
    errors: &mut Vec<DiscoveryError>,
    seen_names: &mut HashSet<String>,
) {
    if !dir.exists() {
        debug!(path = %dir.display(), "skills directory not found, skipping");
        return;
    }

    if !dir.is_dir() {
        debug!(path = %dir.display(), "skills path is not a directory, skipping");
        return;
    }

    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) => {
            warn!(
                path = %dir.display(),
                error = %e,
                "failed to read skills directory"
            );
            return;
        }
    };

    for entry in entries.filter_map(Result::ok) {
        let skill_dir = entry.path();

        // Each skill is a subdirectory containing SKILL.md.
        if !skill_dir.is_dir() {
            continue;
        }

        let skill_md_path = skill_dir.join("SKILL.md");
        if !skill_md_path.exists() {
            debug!(
                path = %skill_dir.display(),
                "no SKILL.md found, skipping"
            );
            continue;
        }

        // Read and parse SKILL.md.
        let content = match fs::read_to_string(&skill_md_path) {
            Ok(content) => content,
            Err(e) => {
                let skill_name = skill_dir
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("unknown")
                    .to_string();
                warn!(
                    path = %skill_md_path.display(),
                    error = %e,
                    "failed to read SKILL.md"
                );
                errors.push(DiscoveryError {
                    name: skill_name,
                    path: skill_md_path.clone(),
                    error: SkillError::InvalidYaml(format!("IO error: {e}")),
                });
                continue;
            }
        };

        match parse_skill_md(&content, skill_dir.clone(), location) {
            Ok(metadata) => {
                // Check for duplicates (prefer first match per spec Section 5.2).
                if seen_names.contains(&metadata.name) {
                    debug!(
                        name = %metadata.name,
                        path = %skill_dir.display(),
                        "duplicate skill name, skipping"
                    );
                    continue;
                }

                seen_names.insert(metadata.name.clone());
                skills.push(metadata);
            }
            Err(e) => {
                let skill_name = skill_dir
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("unknown")
                    .to_string();
                warn!(
                    path = %skill_md_path.display(),
                    error = %e,
                    "failed to parse SKILL.md"
                );
                errors.push(DiscoveryError {
                    name: skill_name,
                    path: skill_md_path.clone(),
                    error: e,
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn make_skill(dir: &Path, name: &str, description: &str) {
        let skill_dir = dir.join(name);
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("SKILL.md"),
            format!(
                "---\nname: {}\ndescription: {}\n---\n\nInstructions.",
                name, description
            ),
        )
        .unwrap();
    }

    fn test_config(skills_dir: &Path) -> Config {
        let mut config = Config::default();
        config.skills_dirs = vec![skills_dir.to_path_buf()];
        config.skills_sync_on_start = false;
        config
    }

    #[test]
    fn discovers_skills_in_directory() {
        let tmp = TempDir::new().unwrap();
        let skills_dir = tmp.path().join(".agent/skills");
        fs::create_dir_all(&skills_dir).unwrap();

        make_skill(&skills_dir, "pdf-processing", "Extract text from PDFs.");
        make_skill(
            &skills_dir,
            "code-review",
            "Review code for best practices.",
        );

        let config = test_config(&skills_dir);
        let result = discover_skills(&config, tmp.path());

        assert_eq!(result.skills.len(), 2);
        assert!(result.errors.is_empty());

        let names: Vec<_> = result.skills.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"pdf-processing"));
        assert!(names.contains(&"code-review"));
    }

    #[test]
    fn skips_nonexistent_directory() {
        let tmp = TempDir::new().unwrap();
        let config = test_config(&tmp.path().join("nonexistent"));
        let result = discover_skills(&config, tmp.path());

        assert!(result.skills.is_empty());
        assert!(result.errors.is_empty());
    }

    #[test]
    fn skips_directories_without_skill_md() {
        let tmp = TempDir::new().unwrap();
        let skills_dir = tmp.path().join("skills");
        fs::create_dir_all(&skills_dir).unwrap();

        // Create a directory without SKILL.md.
        let bad_skill = skills_dir.join("no-skill-md");
        fs::create_dir_all(&bad_skill).unwrap();
        fs::write(bad_skill.join("README.md"), "Not a skill").unwrap();

        // Create a valid skill.
        make_skill(&skills_dir, "valid-skill", "A valid skill.");

        let config = test_config(&skills_dir);
        let result = discover_skills(&config, tmp.path());

        assert_eq!(result.skills.len(), 1);
        assert_eq!(result.skills[0].name, "valid-skill");
    }

    #[test]
    fn records_parse_errors() {
        let tmp = TempDir::new().unwrap();
        let skills_dir = tmp.path().join("skills");
        fs::create_dir_all(&skills_dir).unwrap();

        // Create a skill with invalid frontmatter.
        let bad_skill = skills_dir.join("bad-skill");
        fs::create_dir_all(&bad_skill).unwrap();
        fs::write(
            bad_skill.join("SKILL.md"),
            "---\nname: INVALID-NAME\ndescription: Valid description.\n---\n",
        )
        .unwrap();

        // Create a valid skill.
        make_skill(&skills_dir, "good-skill", "A good skill.");

        let config = test_config(&skills_dir);
        let result = discover_skills(&config, tmp.path());

        assert_eq!(result.skills.len(), 1);
        assert_eq!(result.skills[0].name, "good-skill");
        assert_eq!(result.errors.len(), 1);
        assert_eq!(result.errors[0].name, "bad-skill");
    }

    #[test]
    fn deduplicates_by_name_preferring_first() {
        let tmp = TempDir::new().unwrap();

        // Create two directories with the same skill name.
        let dir1 = tmp.path().join("dir1");
        let dir2 = tmp.path().join("dir2");
        fs::create_dir_all(&dir1).unwrap();
        fs::create_dir_all(&dir2).unwrap();

        make_skill(&dir1, "my-skill", "First occurrence.");
        make_skill(&dir2, "my-skill", "Second occurrence.");

        let mut config = Config::default();
        config.skills_dirs = vec![dir1.clone(), dir2.clone()];
        config.skills_sync_on_start = false;

        let result = discover_skills(&config, tmp.path());

        assert_eq!(result.skills.len(), 1);
        assert_eq!(result.skills[0].name, "my-skill");
        assert_eq!(result.skills[0].description, "First occurrence.");
        assert_eq!(result.skills[0].path, dir1.join("my-skill"));
    }

    #[test]
    fn assigns_project_location_for_workspace_relative() {
        let tmp = TempDir::new().unwrap();
        let skills_dir = tmp.path().join(".agent/skills");
        fs::create_dir_all(&skills_dir).unwrap();

        make_skill(&skills_dir, "local-skill", "A local skill.");

        let config = test_config(&skills_dir);
        let result = discover_skills(&config, tmp.path());

        assert_eq!(result.skills.len(), 1);
        assert_eq!(result.skills[0].location, SkillLocation::Project);
    }

    #[test]
    fn assigns_global_location_for_outside_workspace() {
        let workspace = TempDir::new().unwrap();
        let global_dir = TempDir::new().unwrap();
        let skills_dir = global_dir.path().join("skills");
        fs::create_dir_all(&skills_dir).unwrap();

        make_skill(&skills_dir, "global-skill", "A global skill.");

        let config = test_config(&skills_dir);
        let result = discover_skills(&config, workspace.path());

        assert_eq!(result.skills.len(), 1);
        assert_eq!(result.skills[0].location, SkillLocation::Global);
    }

    #[test]
    fn scans_synced_builtin_dir() {
        let tmp = TempDir::new().unwrap();
        let sync_dir = tmp.path().join("synced-skills");
        fs::create_dir_all(&sync_dir).unwrap();

        make_skill(&sync_dir, "builtin-skill", "A built-in skill.");

        let mut config = Config::default();
        config.skills_dirs = vec![];
        config.skills_sync_on_start = true;
        config.skills_sync_dir = sync_dir;

        let result = discover_skills(&config, tmp.path());

        assert_eq!(result.skills.len(), 1);
        assert_eq!(result.skills[0].name, "builtin-skill");
        assert_eq!(result.skills[0].location, SkillLocation::Global);
    }

    #[test]
    fn handles_missing_description() {
        let tmp = TempDir::new().unwrap();
        let skills_dir = tmp.path().join("skills");
        fs::create_dir_all(&skills_dir).unwrap();

        // Create a skill without description.
        let bad_skill = skills_dir.join("no-desc");
        fs::create_dir_all(&bad_skill).unwrap();
        fs::write(
            bad_skill.join("SKILL.md"),
            "---\nname: no-desc\n---\n\nNo description field.",
        )
        .unwrap();

        let config = test_config(&skills_dir);
        let result = discover_skills(&config, tmp.path());

        assert!(result.skills.is_empty());
        assert_eq!(result.errors.len(), 1);
        assert_eq!(result.errors[0].name, "no-desc");
    }

    #[test]
    fn handles_empty_description() {
        let tmp = TempDir::new().unwrap();
        let skills_dir = tmp.path().join("skills");
        fs::create_dir_all(&skills_dir).unwrap();

        // Create a skill with empty description.
        let bad_skill = skills_dir.join("empty-desc");
        fs::create_dir_all(&bad_skill).unwrap();
        fs::write(
            bad_skill.join("SKILL.md"),
            "---\nname: empty-desc\ndescription: \"\"\n---\n",
        )
        .unwrap();

        let config = test_config(&skills_dir);
        let result = discover_skills(&config, tmp.path());

        assert!(result.skills.is_empty());
        assert_eq!(result.errors.len(), 1);
        assert_eq!(result.errors[0].name, "empty-desc");
    }
}
