//! Built-in skill sync from repo to daemon data directory.
//!
//! Implements spec Section 5.1: On daemon start, sync built-in skills from
//! `skills_builtin_dir` to `skills_sync_dir` (if enabled).

use std::fs;
use std::io;
use std::path::Path;
use tracing::{info, warn};

/// Synchronize built-in skills from the source directory to the target directory.
///
/// Per spec Section 5.2: Built-in sync failure is logged and the function
/// returns without error, allowing the daemon to continue using the repo
/// directory directly.
///
/// # Arguments
/// * `src_dir` - Source directory containing built-in skills (e.g., `skills/`)
/// * `dst_dir` - Target directory for synced skills (e.g., `~/.local/share/loopd/skills`)
///
/// # Returns
/// * `Ok(true)` - Sync completed successfully
/// * `Ok(false)` - Sync was skipped (source doesn't exist or other non-fatal condition)
pub fn sync_builtin_skills(src_dir: &Path, dst_dir: &Path) -> io::Result<bool> {
    // Check if source directory exists.
    if !src_dir.exists() {
        info!(
            src = %src_dir.display(),
            "built-in skills directory not found, skipping sync"
        );
        return Ok(false);
    }

    if !src_dir.is_dir() {
        warn!(
            src = %src_dir.display(),
            "built-in skills path is not a directory, skipping sync"
        );
        return Ok(false);
    }

    // Create target directory if it doesn't exist.
    if let Err(e) = fs::create_dir_all(dst_dir) {
        warn!(
            dst = %dst_dir.display(),
            error = %e,
            "failed to create skills sync directory"
        );
        return Err(e);
    }

    // Recursively copy skill directories.
    match copy_dir_recursive(src_dir, dst_dir) {
        Ok(count) => {
            info!(
                src = %src_dir.display(),
                dst = %dst_dir.display(),
                count,
                "synced built-in skills"
            );
            Ok(count > 0)
        }
        Err(e) => {
            warn!(
                src = %src_dir.display(),
                dst = %dst_dir.display(),
                error = %e,
                "failed to sync built-in skills"
            );
            Err(e)
        }
    }
}

/// Recursively copy a directory tree.
///
/// Returns the number of files copied.
fn copy_dir_recursive(src: &Path, dst: &Path) -> io::Result<usize> {
    let mut count = 0;

    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let src_path = entry.path();
        let file_name = entry.file_name();
        let dst_path = dst.join(&file_name);

        let file_type = entry.file_type()?;

        if file_type.is_dir() {
            // Create destination directory and recurse.
            fs::create_dir_all(&dst_path)?;
            count += copy_dir_recursive(&src_path, &dst_path)?;
        } else if file_type.is_file() {
            // Copy file, overwriting if it exists.
            fs::copy(&src_path, &dst_path)?;
            count += 1;
        }
        // Skip symlinks and other special files.
    }

    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn sync_copies_skill_directories() {
        let src = TempDir::new().unwrap();
        let dst = TempDir::new().unwrap();

        // Create a skill directory structure.
        let skill_dir = src.path().join("my-skill");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: my-skill\ndescription: Test skill\n---\n\nInstructions.",
        )
        .unwrap();
        fs::create_dir_all(skill_dir.join("references")).unwrap();
        fs::write(skill_dir.join("references/doc.md"), "Reference doc.").unwrap();

        // Sync.
        let result = sync_builtin_skills(src.path(), dst.path()).unwrap();
        assert!(result);

        // Verify copied structure.
        assert!(dst.path().join("my-skill/SKILL.md").exists());
        assert!(dst.path().join("my-skill/references/doc.md").exists());
    }

    #[test]
    fn sync_skips_nonexistent_source() {
        let dst = TempDir::new().unwrap();
        let src = dst.path().join("nonexistent");

        let result = sync_builtin_skills(&src, dst.path()).unwrap();
        assert!(!result);
    }

    #[test]
    fn sync_skips_file_as_source() {
        let dir = TempDir::new().unwrap();
        let src_file = dir.path().join("not-a-dir");
        fs::write(&src_file, "I'm a file").unwrap();
        let dst = dir.path().join("dst");

        let result = sync_builtin_skills(&src_file, &dst).unwrap();
        assert!(!result);
    }

    #[test]
    fn sync_creates_destination_directory() {
        let src = TempDir::new().unwrap();
        let dst_base = TempDir::new().unwrap();
        let dst = dst_base.path().join("nested/skills");

        // Create a skill.
        let skill_dir = src.path().join("test-skill");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(skill_dir.join("SKILL.md"), "content").unwrap();

        // Sync to nested path.
        let result = sync_builtin_skills(src.path(), &dst).unwrap();
        assert!(result);
        assert!(dst.join("test-skill/SKILL.md").exists());
    }

    #[test]
    fn sync_overwrites_existing_files() {
        let src = TempDir::new().unwrap();
        let dst = TempDir::new().unwrap();

        // Create source skill.
        let skill_dir = src.path().join("skill");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(skill_dir.join("SKILL.md"), "new content").unwrap();

        // Create existing destination with old content.
        let dst_skill = dst.path().join("skill");
        fs::create_dir_all(&dst_skill).unwrap();
        fs::write(dst_skill.join("SKILL.md"), "old content").unwrap();

        // Sync should overwrite.
        let result = sync_builtin_skills(src.path(), dst.path()).unwrap();
        assert!(result);

        let content = fs::read_to_string(dst.path().join("skill/SKILL.md")).unwrap();
        assert_eq!(content, "new content");
    }

    #[test]
    fn sync_returns_zero_for_empty_source() {
        let src = TempDir::new().unwrap();
        let dst = TempDir::new().unwrap();

        // Empty source directory.
        let result = sync_builtin_skills(src.path(), dst.path()).unwrap();
        assert!(!result);
    }
}
