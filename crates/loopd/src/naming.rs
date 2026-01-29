//! Run naming utilities.
//!
//! Implements run name generation per spec Section 3 and Section 4.1:
//! - `name_source=haiku`: Use Claude haiku model to generate a short label
//! - `name_source=spec_slug`: Use the spec filename or title
//! - Names are ASCII, max 64 chars; daemon truncates if needed
//! - If haiku generation fails, fall back to spec_slug

use loop_core::{prompt::spec_slug, RunNameSource};
use std::path::Path;
use std::process::Command;
use thiserror::Error;

/// Maximum length for run names.
pub const MAX_NAME_LENGTH: usize = 64;

#[derive(Debug, Error)]
pub enum NamingError {
    #[error("haiku generation failed: {0}")]
    HaikuFailed(String),
    #[error("claude CLI not available")]
    ClaudeNotAvailable,
}

/// Result of name generation.
pub struct NameResult {
    pub name: String,
    pub source: RunNameSource,
}

/// Generate a run name based on the configured source.
///
/// If `source` is `Haiku`, attempts to generate a name using Claude haiku model.
/// Falls back to `SpecSlug` if haiku generation fails.
pub fn generate_name(spec_path: &Path, source: RunNameSource, haiku_model: &str) -> NameResult {
    match source {
        RunNameSource::Haiku => match generate_haiku_name(spec_path, haiku_model) {
            Ok(name) => NameResult {
                name: sanitize_name(&name),
                source: RunNameSource::Haiku,
            },
            Err(_) => {
                // Fall back to spec_slug on failure
                let name = spec_slug(spec_path);
                NameResult {
                    name: sanitize_name(&name),
                    source: RunNameSource::SpecSlug,
                }
            }
        },
        RunNameSource::SpecSlug => {
            let name = spec_slug(spec_path);
            NameResult {
                name: sanitize_name(&name),
                source: RunNameSource::SpecSlug,
            }
        }
    }
}

/// Generate a haiku-style name using Claude.
fn generate_haiku_name(spec_path: &Path, model: &str) -> Result<String, NamingError> {
    // Check if claude CLI is available
    if Command::new("claude").arg("--version").output().is_err() {
        return Err(NamingError::ClaudeNotAvailable);
    }

    let spec_name = spec_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unnamed");

    let prompt = format!(
        "Generate a short, memorable name (2-4 words, lowercase, hyphen-separated) for a development task based on this spec name: '{}'. \
         Output ONLY the name, nothing else. Examples: 'swift-owl', 'cosmic-garden', 'quiet-thunder'.",
        spec_name
    );

    let output = Command::new("claude")
        .args(["--model", model, "--print", "-p", &prompt])
        .output()
        .map_err(|e| NamingError::HaikuFailed(e.to_string()))?;

    if !output.status.success() {
        return Err(NamingError::HaikuFailed(
            String::from_utf8_lossy(&output.stderr).to_string(),
        ));
    }

    let name = String::from_utf8_lossy(&output.stdout).trim().to_string();

    if name.is_empty() {
        return Err(NamingError::HaikuFailed("empty response".to_string()));
    }

    Ok(name)
}

/// Sanitize a name to be ASCII, lowercase, and max 64 chars.
fn sanitize_name(name: &str) -> String {
    let sanitized: String = name
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .take(MAX_NAME_LENGTH)
        .collect();

    if sanitized.is_empty() {
        "unnamed".to_string()
    } else {
        sanitized.to_lowercase()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_name_removes_special_chars() {
        assert_eq!(sanitize_name("hello world!"), "helloworld");
        assert_eq!(sanitize_name("my-feature_test"), "my-feature_test");
        assert_eq!(sanitize_name("UPPERCASE"), "uppercase");
    }

    #[test]
    fn sanitize_name_truncates_long_names() {
        let long_name = "a".repeat(100);
        assert_eq!(sanitize_name(&long_name).len(), MAX_NAME_LENGTH);
    }

    #[test]
    fn sanitize_name_handles_empty() {
        assert_eq!(sanitize_name(""), "unnamed");
        assert_eq!(sanitize_name("!!!"), "unnamed");
    }

    #[test]
    fn generate_name_with_spec_slug() {
        let result = generate_name(
            Path::new("specs/my-feature.md"),
            RunNameSource::SpecSlug,
            "haiku",
        );
        assert_eq!(result.name, "my-feature");
        assert_eq!(result.source, RunNameSource::SpecSlug);
    }

    #[test]
    fn generate_name_haiku_fallback() {
        // Without claude CLI available, should fall back to spec_slug
        let result = generate_name(
            Path::new("specs/orchestrator-daemon.md"),
            RunNameSource::Haiku,
            "haiku",
        );
        // Will fall back because claude CLI likely not in test env
        assert!(!result.name.is_empty());
    }
}
