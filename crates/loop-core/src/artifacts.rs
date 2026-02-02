//! Artifact mirroring for the orchestrator daemon.
//!
//! Implements artifact mirroring based on `artifact_mode` config:
//! - `workspace`: Store in workspace only
//! - `global`: Store in global only
//! - `mirror`: Store in both (default)
//!
//! See spec Section 3.2, Section 7.1, Section 8.2.

use crate::types::{Artifact, ArtifactLocation, ArtifactMode, Id};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ArtifactError {
    #[error("io error: {0}")]
    Io(#[from] io::Error),
    #[error("source file not found: {0}")]
    SourceNotFound(PathBuf),
}

pub type Result<T> = std::result::Result<T, ArtifactError>;

/// Generate the workspace run directory path.
///
/// Spec Section 3.2: `<workspace_root>/logs/loop/run-<run_id>/`
pub fn workspace_run_dir(workspace_root: &Path, run_id: &Id) -> PathBuf {
    workspace_root
        .join("logs/loop")
        .join(format!("run-{run_id}"))
}

/// Generate the global run directory path.
///
/// Spec Section 3.2: `~/.local/share/loopd/runs/run-<run_id>/`
pub fn global_run_dir(global_log_dir: &Path, run_id: &Id) -> PathBuf {
    global_log_dir.join("runs").join(format!("run-{run_id}"))
}

/// Compute SHA256 checksum of file contents.
fn compute_checksum(path: &Path) -> Result<String> {
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 8192];

    loop {
        let bytes_read = file.read(&mut buffer)?;
        if bytes_read == 0 {
            break;
        }
        hasher.update(&buffer[..bytes_read]);
    }

    Ok(format!("{:x}", hasher.finalize()))
}

/// Mirror an artifact file based on the artifact mode.
///
/// Returns a list of Artifact records to store in `SQLite`.
/// The source file should already exist in the workspace directory.
pub fn mirror_artifact(
    run_id: &Id,
    kind: &str,
    workspace_path: &Path,
    global_log_dir: &Path,
    mode: ArtifactMode,
) -> Result<Vec<Artifact>> {
    if !workspace_path.exists() {
        return Err(ArtifactError::SourceNotFound(workspace_path.to_path_buf()));
    }

    let mut artifacts = Vec::new();
    let checksum = compute_checksum(workspace_path)?;

    // Get the relative path within the run directory for the global copy.
    let filename = workspace_path.file_name().map_or_else(
        || "artifact".to_string(),
        |s| s.to_string_lossy().to_string(),
    );

    match mode {
        ArtifactMode::Workspace => {
            // Workspace only - just create the artifact record.
            artifacts.push(Artifact {
                id: Id::new(),
                run_id: run_id.clone(),
                kind: kind.to_string(),
                location: ArtifactLocation::Workspace,
                path: workspace_path.to_string_lossy().to_string(),
                checksum: Some(checksum),
            });
        }
        ArtifactMode::Global => {
            // Global only - copy to global dir and create record.
            let global_dir = global_run_dir(global_log_dir, run_id);
            fs::create_dir_all(&global_dir)?;
            let global_path = global_dir.join(&filename);
            fs::copy(workspace_path, &global_path)?;

            artifacts.push(Artifact {
                id: Id::new(),
                run_id: run_id.clone(),
                kind: kind.to_string(),
                location: ArtifactLocation::Global,
                path: global_path.to_string_lossy().to_string(),
                checksum: Some(checksum),
            });
        }
        ArtifactMode::Mirror => {
            // Both workspace and global - create both records.
            artifacts.push(Artifact {
                id: Id::new(),
                run_id: run_id.clone(),
                kind: kind.to_string(),
                location: ArtifactLocation::Workspace,
                path: workspace_path.to_string_lossy().to_string(),
                checksum: Some(checksum.clone()),
            });

            let global_dir = global_run_dir(global_log_dir, run_id);
            fs::create_dir_all(&global_dir)?;
            let global_path = global_dir.join(&filename);
            fs::copy(workspace_path, &global_path)?;

            artifacts.push(Artifact {
                id: Id::new(),
                run_id: run_id.clone(),
                kind: kind.to_string(),
                location: ArtifactLocation::Global,
                path: global_path.to_string_lossy().to_string(),
                checksum: Some(checksum),
            });
        }
    }

    Ok(artifacts)
}

/// Mirror a file that doesn't exist yet - write to workspace and optionally to global.
///
/// Returns a list of Artifact records to store in `SQLite`.
pub fn write_and_mirror_artifact(
    run_id: &Id,
    kind: &str,
    filename: &str,
    content: &[u8],
    workspace_root: &Path,
    global_log_dir: &Path,
    mode: ArtifactMode,
) -> Result<Vec<Artifact>> {
    let mut artifacts = Vec::new();

    // Compute checksum from content.
    let mut hasher = Sha256::new();
    hasher.update(content);
    let checksum = format!("{:x}", hasher.finalize());

    let workspace_dir = workspace_run_dir(workspace_root, run_id);
    fs::create_dir_all(&workspace_dir)?;
    let workspace_path = workspace_dir.join(filename);

    match mode {
        ArtifactMode::Workspace => {
            // Write to workspace only.
            let mut file = fs::File::create(&workspace_path)?;
            file.write_all(content)?;

            artifacts.push(Artifact {
                id: Id::new(),
                run_id: run_id.clone(),
                kind: kind.to_string(),
                location: ArtifactLocation::Workspace,
                path: workspace_path.to_string_lossy().to_string(),
                checksum: Some(checksum),
            });
        }
        ArtifactMode::Global => {
            // Write to global only.
            let global_dir = global_run_dir(global_log_dir, run_id);
            fs::create_dir_all(&global_dir)?;
            let global_path = global_dir.join(filename);

            let mut file = fs::File::create(&global_path)?;
            file.write_all(content)?;

            artifacts.push(Artifact {
                id: Id::new(),
                run_id: run_id.clone(),
                kind: kind.to_string(),
                location: ArtifactLocation::Global,
                path: global_path.to_string_lossy().to_string(),
                checksum: Some(checksum),
            });
        }
        ArtifactMode::Mirror => {
            // Write to both workspace and global.
            let mut file = fs::File::create(&workspace_path)?;
            file.write_all(content)?;

            artifacts.push(Artifact {
                id: Id::new(),
                run_id: run_id.clone(),
                kind: kind.to_string(),
                location: ArtifactLocation::Workspace,
                path: workspace_path.to_string_lossy().to_string(),
                checksum: Some(checksum.clone()),
            });

            let global_dir = global_run_dir(global_log_dir, run_id);
            fs::create_dir_all(&global_dir)?;
            let global_path = global_dir.join(filename);

            let mut file = fs::File::create(&global_path)?;
            file.write_all(content)?;

            artifacts.push(Artifact {
                id: Id::new(),
                run_id: run_id.clone(),
                kind: kind.to_string(),
                location: ArtifactLocation::Global,
                path: global_path.to_string_lossy().to_string(),
                checksum: Some(checksum),
            });
        }
    }

    Ok(artifacts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn setup_test_dirs() -> (TempDir, TempDir) {
        let workspace = TempDir::new().unwrap();
        let global = TempDir::new().unwrap();
        (workspace, global)
    }

    #[test]
    fn workspace_run_dir_follows_spec() {
        let workspace = PathBuf::from("/workspace");
        let run_id = Id::from_string("abc123");
        let dir = workspace_run_dir(&workspace, &run_id);
        assert_eq!(dir, PathBuf::from("/workspace/logs/loop/run-abc123"));
    }

    #[test]
    fn global_run_dir_follows_spec() {
        let global = PathBuf::from("/home/user/.local/share/loopd");
        let run_id = Id::from_string("abc123");
        let dir = global_run_dir(&global, &run_id);
        assert_eq!(
            dir,
            PathBuf::from("/home/user/.local/share/loopd/runs/run-abc123")
        );
    }

    #[test]
    fn write_and_mirror_workspace_only() {
        let (workspace, global) = setup_test_dirs();
        let run_id = Id::from_string("test-run");
        let content = b"test content";

        let artifacts = write_and_mirror_artifact(
            &run_id,
            "prompt",
            "prompt.txt",
            content,
            workspace.path(),
            global.path(),
            ArtifactMode::Workspace,
        )
        .unwrap();

        assert_eq!(artifacts.len(), 1);
        assert_eq!(artifacts[0].location, ArtifactLocation::Workspace);
        assert!(artifacts[0].checksum.is_some());

        // Verify file exists in workspace.
        let ws_path = workspace_run_dir(workspace.path(), &run_id).join("prompt.txt");
        assert!(ws_path.exists());

        // Verify file does NOT exist in global.
        let gl_path = global_run_dir(global.path(), &run_id).join("prompt.txt");
        assert!(!gl_path.exists());
    }

    #[test]
    fn write_and_mirror_global_only() {
        let (workspace, global) = setup_test_dirs();
        let run_id = Id::from_string("test-run");
        let content = b"test content";

        let artifacts = write_and_mirror_artifact(
            &run_id,
            "prompt",
            "prompt.txt",
            content,
            workspace.path(),
            global.path(),
            ArtifactMode::Global,
        )
        .unwrap();

        assert_eq!(artifacts.len(), 1);
        assert_eq!(artifacts[0].location, ArtifactLocation::Global);

        // Verify file does NOT exist in workspace.
        let ws_path = workspace_run_dir(workspace.path(), &run_id).join("prompt.txt");
        assert!(!ws_path.exists());

        // Verify file exists in global.
        let gl_path = global_run_dir(global.path(), &run_id).join("prompt.txt");
        assert!(gl_path.exists());
    }

    #[test]
    fn write_and_mirror_both() {
        let (workspace, global) = setup_test_dirs();
        let run_id = Id::from_string("test-run");
        let content = b"test content";

        let artifacts = write_and_mirror_artifact(
            &run_id,
            "prompt",
            "prompt.txt",
            content,
            workspace.path(),
            global.path(),
            ArtifactMode::Mirror,
        )
        .unwrap();

        assert_eq!(artifacts.len(), 2);
        assert!(artifacts
            .iter()
            .any(|a| a.location == ArtifactLocation::Workspace));
        assert!(artifacts
            .iter()
            .any(|a| a.location == ArtifactLocation::Global));

        // Verify both checksums are the same.
        assert_eq!(artifacts[0].checksum, artifacts[1].checksum);

        // Verify files exist in both locations.
        let ws_path = workspace_run_dir(workspace.path(), &run_id).join("prompt.txt");
        let gl_path = global_run_dir(global.path(), &run_id).join("prompt.txt");
        assert!(ws_path.exists());
        assert!(gl_path.exists());

        // Verify contents match.
        let ws_content = fs::read_to_string(&ws_path).unwrap();
        let gl_content = fs::read_to_string(&gl_path).unwrap();
        assert_eq!(ws_content, gl_content);
    }

    #[test]
    fn mirror_existing_artifact() {
        let (workspace, global) = setup_test_dirs();
        let run_id = Id::from_string("test-run");

        // Create a file in workspace first.
        let ws_dir = workspace_run_dir(workspace.path(), &run_id);
        fs::create_dir_all(&ws_dir).unwrap();
        let ws_path = ws_dir.join("iter-01.log");
        fs::write(&ws_path, "iteration output").unwrap();

        let artifacts = mirror_artifact(
            &run_id,
            "output",
            &ws_path,
            global.path(),
            ArtifactMode::Mirror,
        )
        .unwrap();

        assert_eq!(artifacts.len(), 2);

        // Verify global copy exists.
        let gl_path = global_run_dir(global.path(), &run_id).join("iter-01.log");
        assert!(gl_path.exists());

        let gl_content = fs::read_to_string(&gl_path).unwrap();
        assert_eq!(gl_content, "iteration output");
    }

    #[test]
    fn mirror_nonexistent_file_fails() {
        let (workspace, global) = setup_test_dirs();
        let run_id = Id::from_string("test-run");
        let fake_path = workspace.path().join("nonexistent.txt");

        let result = mirror_artifact(
            &run_id,
            "output",
            &fake_path,
            global.path(),
            ArtifactMode::Mirror,
        );

        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            ArtifactError::SourceNotFound(_)
        ));
    }

    #[test]
    fn checksum_is_computed_correctly() {
        let (workspace, global) = setup_test_dirs();
        let run_id = Id::from_string("test-run");
        let content = b"hello world";

        // Known SHA256 of "hello world"
        let expected_checksum = "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9";

        let artifacts = write_and_mirror_artifact(
            &run_id,
            "test",
            "test.txt",
            content,
            workspace.path(),
            global.path(),
            ArtifactMode::Workspace,
        )
        .unwrap();

        assert_eq!(artifacts[0].checksum.as_deref(), Some(expected_checksum));
    }
}
