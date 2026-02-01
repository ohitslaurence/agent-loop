//! Configuration parsing for the orchestrator daemon.
//!
//! Matches the key=value format from `.loop/config` used by `bin/loop`.
//! Precedence: CLI flags > `--config` file > `.loop/config` > defaults.

use crate::types::{
    ArtifactMode, CompletionMode, MergeStrategy, QueuePolicy, RunNameSource, WorktreeProvider,
};
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("failed to read config file: {0}")]
    ReadError(#[from] std::io::Error),
    #[error("invalid config line: {0}")]
    InvalidLine(String),
    #[error("invalid boolean value for {key}: {value}")]
    InvalidBool { key: String, value: String },
    #[error("invalid integer value for {key}: {value}")]
    InvalidInt { key: String, value: String },
    #[error("unknown config key: {0}")]
    UnknownKey(String),
}

/// Daemon and run configuration.
///
/// Field names match the config keys from `bin/loop` (Section 4.3 of spec).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Config {
    // Directories
    pub specs_dir: PathBuf,
    pub plans_dir: PathBuf,
    pub log_dir: PathBuf,
    pub global_log_dir: PathBuf,

    // Model and iterations
    pub model: String,
    pub iterations: u32,

    // Completion and mode
    pub completion_mode: CompletionMode,

    // Reviewer
    pub reviewer: bool,

    // Prompt customization
    pub prompt_file: Option<PathBuf>,
    pub context_files: Vec<PathBuf>,

    // Verification
    pub verify_cmds: Vec<String>,
    pub verify_timeout_sec: u32,

    // Claude CLI settings
    pub claude_timeout_sec: u32,
    pub claude_retries: u32,
    pub claude_retry_backoff_sec: u32,

    // Artifacts
    pub artifact_mode: ArtifactMode,

    // Run naming
    pub run_naming_mode: RunNameSource,
    pub run_naming_model: String,

    // Worktree and merge
    pub base_branch: Option<String>,
    pub run_branch_prefix: String,
    pub merge_target_branch: Option<String>,
    pub merge_strategy: MergeStrategy,
    pub worktree_path_template: String,

    // Local scaling (Section 4.3, 5.3)
    pub queue_policy: QueuePolicy,

    // Worktree provider (worktrunk-integration.md Section 4.1)
    pub worktree_provider: WorktreeProvider,
    pub worktrunk_bin: PathBuf,
    pub worktrunk_config_path: Option<PathBuf>,
    pub worktrunk_copy_ignored: bool,
    /// Remove worktree after run completes (worktrunk-integration.md Section 5.4).
    /// Default: false.
    pub worktree_cleanup: bool,

    // Postmortem settings (postmortem-analysis.md Section 4)
    /// Write summary.json after run ends (default: true).
    pub summary_json: bool,
    /// Run postmortem analysis after run ends (default: true).
    pub postmortem: bool,

    // Skills settings (open-skills-orchestration.md Section 4.1)
    /// Enable skill discovery and selection (default: false).
    pub skills_enabled: bool,
    /// Directory containing built-in skills committed to repo (default: skills/).
    pub skills_builtin_dir: PathBuf,
    /// Directory to sync built-in skills to on daemon start (default: ~/.local/share/loopd/skills).
    pub skills_sync_dir: PathBuf,
    /// Sync built-in skills to skills_sync_dir on start (default: true).
    pub skills_sync_on_start: bool,
    /// Directories to scan for skills in priority order (default: OpenSkills order).
    pub skills_dirs: Vec<PathBuf>,
    /// Maximum skills to select for implementation steps (default: 2).
    pub skills_max_selected_impl: u8,
    /// Maximum skills to select for review steps (default: 1).
    pub skills_max_selected_review: u8,
    /// Include references/ directory content when loading skills (default: false).
    pub skills_load_references: bool,
    /// Maximum characters to load from SKILL.md body (default: 20000).
    pub skills_max_body_chars: usize,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            specs_dir: PathBuf::from("specs"),
            plans_dir: PathBuf::from("specs/planning"),
            log_dir: PathBuf::from("logs/loop"),
            global_log_dir: dirs::data_local_dir().map_or_else(|| PathBuf::from("~/.local/share/loopd"), |d| d.join("loopd")),
            model: "opus".to_string(),
            iterations: 50,
            completion_mode: CompletionMode::Trailing,
            reviewer: true,
            prompt_file: None,
            context_files: Vec::new(),
            verify_cmds: Vec::new(),
            verify_timeout_sec: 0,
            claude_timeout_sec: 600,
            claude_retries: 0,
            claude_retry_backoff_sec: 5,
            artifact_mode: ArtifactMode::Mirror,
            run_naming_mode: RunNameSource::Haiku,
            run_naming_model: "haiku".to_string(),
            base_branch: None,
            run_branch_prefix: "run/".to_string(),
            merge_target_branch: None,
            merge_strategy: MergeStrategy::Squash,
            worktree_path_template: "../{{ repo }}.{{ run_branch | sanitize }}".to_string(),
            queue_policy: QueuePolicy::Fifo,
            worktree_provider: WorktreeProvider::Auto,
            worktrunk_bin: PathBuf::from("wt"),
            worktrunk_config_path: None,
            worktrunk_copy_ignored: false,
            worktree_cleanup: true,
            summary_json: true,
            postmortem: true,
            // Skills defaults (open-skills-orchestration.md Section 4.1)
            skills_enabled: false,
            skills_builtin_dir: PathBuf::from("skills"),
            skills_sync_dir: dirs::data_local_dir()
                .map_or_else(|| PathBuf::from("~/.local/share/loopd/skills"), |d| d.join("loopd/skills")),
            skills_sync_on_start: true,
            skills_dirs: vec![
                PathBuf::from(".agent/skills"),
                dirs::home_dir()
                    .map_or_else(|| PathBuf::from("~/.agent/skills"), |h| h.join(".agent/skills")),
                PathBuf::from(".claude/skills"),
                dirs::home_dir()
                    .map_or_else(|| PathBuf::from("~/.claude/skills"), |h| h.join(".claude/skills")),
            ],
            skills_max_selected_impl: 2,
            skills_max_selected_review: 1,
            skills_load_references: false,
            skills_max_body_chars: 20000,
        }
    }
}

impl Config {
    /// Load config from a file, merging with defaults.
    pub fn from_file(path: &Path) -> Result<Self, ConfigError> {
        let mut config = Self::default();
        config.load_file(path)?;
        Ok(config)
    }

    /// Load and merge values from a config file.
    pub fn load_file(&mut self, path: &Path) -> Result<(), ConfigError> {
        let content = std::fs::read_to_string(path)?;
        self.parse_content(&content, path.display().to_string())
    }

    /// Parse config content (key=value format).
    fn parse_content(&mut self, content: &str, source: String) -> Result<(), ConfigError> {
        for line in content.lines() {
            let trimmed = line.trim();

            // Skip empty lines and comments
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }

            // Must contain '='
            let Some((key, value)) = trimmed.split_once('=') else {
                return Err(ConfigError::InvalidLine(line.to_string()));
            };

            let key = key.trim();
            let value = Self::unquote(value.trim());

            self.apply_value(key, &value, &source)?;
        }
        Ok(())
    }

    /// Remove surrounding quotes from a value.
    fn unquote(value: &str) -> String {
        if value.len() >= 2
            && ((value.starts_with('"') && value.ends_with('"'))
                || (value.starts_with('\'') && value.ends_with('\'')))
            {
                return value[1..value.len() - 1].to_string();
            }
        value.to_string()
    }

    /// Apply a single config value.
    fn apply_value(&mut self, key: &str, value: &str, _source: &str) -> Result<(), ConfigError> {
        match key {
            "specs_dir" => self.specs_dir = PathBuf::from(value),
            "plans_dir" => self.plans_dir = PathBuf::from(value),
            "log_dir" => self.log_dir = PathBuf::from(value),
            "global_log_dir" => self.global_log_dir = PathBuf::from(value),
            "model" => self.model = value.to_string(),
            "iterations" => {
                self.iterations = value.parse().map_err(|_| ConfigError::InvalidInt {
                    key: key.to_string(),
                    value: value.to_string(),
                })?;
            }
            "completion_mode" => {
                self.completion_mode = match value {
                    "exact" => CompletionMode::Exact,
                    "trailing" => CompletionMode::Trailing,
                    _ => {
                        return Err(ConfigError::InvalidLine(format!(
                            "completion_mode must be 'exact' or 'trailing', got '{value}'"
                        )))
                    }
                }
            }
            "reviewer" => self.reviewer = Self::parse_bool(key, value)?,
            "prompt_file" => {
                self.prompt_file = if value.is_empty() {
                    None
                } else {
                    Some(PathBuf::from(value))
                }
            }
            "context_files" => {
                self.context_files = value.split_whitespace().map(PathBuf::from).collect();
            }
            "verify_cmds" => {
                // Pipe-separated list of commands
                self.verify_cmds = value
                    .split('|')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
            }
            "verify_timeout_sec" => {
                self.verify_timeout_sec = value.parse().map_err(|_| ConfigError::InvalidInt {
                    key: key.to_string(),
                    value: value.to_string(),
                })?;
            }
            "claude_timeout_sec" => {
                self.claude_timeout_sec = value.parse().map_err(|_| ConfigError::InvalidInt {
                    key: key.to_string(),
                    value: value.to_string(),
                })?;
            }
            "claude_retries" => {
                self.claude_retries = value.parse().map_err(|_| ConfigError::InvalidInt {
                    key: key.to_string(),
                    value: value.to_string(),
                })?;
            }
            "claude_retry_backoff_sec" => {
                self.claude_retry_backoff_sec =
                    value.parse().map_err(|_| ConfigError::InvalidInt {
                        key: key.to_string(),
                        value: value.to_string(),
                    })?;
            }
            "artifact_mode" => {
                self.artifact_mode = match value {
                    "workspace" => ArtifactMode::Workspace,
                    "global" => ArtifactMode::Global,
                    "mirror" => ArtifactMode::Mirror,
                    _ => {
                        return Err(ConfigError::InvalidLine(format!(
                            "artifact_mode must be 'workspace', 'global', or 'mirror', got '{value}'"
                        )))
                    }
                }
            }
            "run_naming_mode" => {
                self.run_naming_mode = match value {
                    "haiku" => RunNameSource::Haiku,
                    "spec_slug" => RunNameSource::SpecSlug,
                    _ => {
                        return Err(ConfigError::InvalidLine(format!(
                            "run_naming_mode must be 'haiku' or 'spec_slug', got '{value}'"
                        )))
                    }
                }
            }
            "run_naming_model" => self.run_naming_model = value.to_string(),
            "base_branch" => self.base_branch = Some(value.to_string()),
            "run_branch_prefix" => self.run_branch_prefix = value.to_string(),
            "merge_target_branch" => self.merge_target_branch = Some(value.to_string()),
            "merge_strategy" => {
                self.merge_strategy = match value {
                    "none" => MergeStrategy::None,
                    "merge" => MergeStrategy::Merge,
                    "squash" => MergeStrategy::Squash,
                    _ => {
                        return Err(ConfigError::InvalidLine(format!(
                            "merge_strategy must be 'none', 'merge', or 'squash', got '{value}'"
                        )))
                    }
                }
            }
            "worktree_path_template" => self.worktree_path_template = value.to_string(),
            "queue_policy" => {
                self.queue_policy = match value {
                    "fifo" => QueuePolicy::Fifo,
                    "newest_first" => QueuePolicy::NewestFirst,
                    _ => {
                        return Err(ConfigError::InvalidLine(format!(
                            "queue_policy must be 'fifo' or 'newest_first', got '{value}'"
                        )))
                    }
                }
            }
            "worktree_provider" => {
                self.worktree_provider = match value {
                    "auto" => WorktreeProvider::Auto,
                    "worktrunk" => WorktreeProvider::Worktrunk,
                    "git" => WorktreeProvider::Git,
                    _ => {
                        return Err(ConfigError::InvalidLine(format!(
                            "worktree_provider must be 'auto', 'worktrunk', or 'git', got '{value}'"
                        )))
                    }
                }
            }
            "worktrunk_bin" => self.worktrunk_bin = PathBuf::from(value),
            "worktrunk_config_path" => self.worktrunk_config_path = Some(PathBuf::from(value)),
            "worktrunk_copy_ignored" => self.worktrunk_copy_ignored = Self::parse_bool(key, value)?,
            "worktree_cleanup" => self.worktree_cleanup = Self::parse_bool(key, value)?,
            "summary_json" => self.summary_json = Self::parse_bool(key, value)?,
            "postmortem" => self.postmortem = Self::parse_bool(key, value)?,
            // Skills settings (open-skills-orchestration.md Section 4.1)
            "skills_enabled" => self.skills_enabled = Self::parse_bool(key, value)?,
            "skills_builtin_dir" => self.skills_builtin_dir = PathBuf::from(value),
            "skills_sync_dir" => self.skills_sync_dir = PathBuf::from(value),
            "skills_sync_on_start" => self.skills_sync_on_start = Self::parse_bool(key, value)?,
            "skills_dirs" => {
                self.skills_dirs = value.split_whitespace().map(PathBuf::from).collect();
            }
            "skills_max_selected_impl" => {
                self.skills_max_selected_impl = value.parse().map_err(|_| ConfigError::InvalidInt {
                    key: key.to_string(),
                    value: value.to_string(),
                })?;
            }
            "skills_max_selected_review" => {
                self.skills_max_selected_review = value.parse().map_err(|_| ConfigError::InvalidInt {
                    key: key.to_string(),
                    value: value.to_string(),
                })?;
            }
            "skills_load_references" => self.skills_load_references = Self::parse_bool(key, value)?,
            "skills_max_body_chars" => {
                self.skills_max_body_chars = value.parse().map_err(|_| ConfigError::InvalidInt {
                    key: key.to_string(),
                    value: value.to_string(),
                })?;
            }
            // Ignored keys from bin/loop that don't apply to daemon
            "mode" | "no_wait" | "no_gum" | "measure_cmd" | "measure_timeout_sec" => {
                // Silently ignore
            }
            _ => {
                // Warn but don't fail for unknown keys (matches bin/loop behavior)
                eprintln!("Warning: unknown config key: {key}");
            }
        }
        Ok(())
    }

    /// Parse a boolean value (matches bin/loop's `normalize_bool`).
    fn parse_bool(key: &str, value: &str) -> Result<bool, ConfigError> {
        match value.to_lowercase().as_str() {
            "true" | "1" | "yes" | "y" | "on" => Ok(true),
            "false" | "0" | "no" | "n" | "off" => Ok(false),
            _ => Err(ConfigError::InvalidBool {
                key: key.to_string(),
                value: value.to_string(),
            }),
        }
    }

    /// Resolve relative paths against a workspace root.
    pub fn resolve_paths(&mut self, workspace_root: &Path) {
        if self.specs_dir.is_relative() {
            self.specs_dir = workspace_root.join(&self.specs_dir);
        }
        if self.plans_dir.is_relative() {
            self.plans_dir = workspace_root.join(&self.plans_dir);
        }
        if self.log_dir.is_relative() {
            self.log_dir = workspace_root.join(&self.log_dir);
        }
        if let Some(ref prompt_file) = self.prompt_file {
            if prompt_file.is_relative() {
                self.prompt_file = Some(workspace_root.join(prompt_file));
            }
        }
        if !self.context_files.is_empty() {
            self.context_files = self
                .context_files
                .iter()
                .map(|path| {
                    if path.is_relative() {
                        workspace_root.join(path)
                    } else {
                        path.clone()
                    }
                })
                .collect();
        }
        // Resolve skills_builtin_dir relative to workspace
        if self.skills_builtin_dir.is_relative() {
            self.skills_builtin_dir = workspace_root.join(&self.skills_builtin_dir);
        }
    }
}

/// Optional dependency for resolving user directories.
mod dirs {
    use std::path::PathBuf;

    pub fn data_local_dir() -> Option<PathBuf> {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
    }

    pub fn home_dir() -> Option<PathBuf> {
        std::env::var_os("HOME").map(PathBuf::from)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_has_expected_values() {
        let config = Config::default();
        assert_eq!(config.model, "opus");
        assert_eq!(config.iterations, 50);
        assert_eq!(config.completion_mode, CompletionMode::Trailing);
        assert!(config.reviewer);
        assert!(config.prompt_file.is_none());
        assert!(config.context_files.is_empty());
        assert_eq!(config.run_naming_mode, RunNameSource::Haiku);
        assert_eq!(config.merge_strategy, MergeStrategy::Squash);
    }

    #[test]
    fn parse_simple_config() {
        let mut config = Config::default();
        let content = r#"
model="sonnet"
iterations=100
reviewer=false
completion_mode=exact
"#;
        config.parse_content(content, "test".into()).unwrap();
        assert_eq!(config.model, "sonnet");
        assert_eq!(config.iterations, 100);
        assert!(!config.reviewer);
        assert_eq!(config.completion_mode, CompletionMode::Exact);
    }

    #[test]
    fn parse_verify_cmds() {
        let mut config = Config::default();
        let content = r#"verify_cmds="cargo test | cargo clippy""#;
        config.parse_content(content, "test".into()).unwrap();
        assert_eq!(config.verify_cmds, vec!["cargo test", "cargo clippy"]);
    }

    #[test]
    fn unquote_removes_quotes() {
        assert_eq!(Config::unquote("\"hello\""), "hello");
        assert_eq!(Config::unquote("'world'"), "world");
        assert_eq!(Config::unquote("noquotes"), "noquotes");
    }

    #[test]
    fn parse_bool_accepts_variants() {
        assert!(Config::parse_bool("test", "true").unwrap());
        assert!(Config::parse_bool("test", "1").unwrap());
        assert!(Config::parse_bool("test", "yes").unwrap());
        assert!(Config::parse_bool("test", "on").unwrap());
        assert!(!Config::parse_bool("test", "false").unwrap());
        assert!(!Config::parse_bool("test", "0").unwrap());
        assert!(!Config::parse_bool("test", "no").unwrap());
        assert!(!Config::parse_bool("test", "off").unwrap());
    }

    #[test]
    fn default_config_has_expected_worktree_provider_values() {
        let config = Config::default();
        assert_eq!(config.worktree_provider, WorktreeProvider::Auto);
        assert_eq!(config.worktrunk_bin, PathBuf::from("wt"));
        assert!(config.worktrunk_config_path.is_none());
        assert!(!config.worktrunk_copy_ignored);
    }

    #[test]
    fn parse_worktree_provider_config() {
        let mut config = Config::default();
        let content = r#"
worktree_provider=worktrunk
worktrunk_bin=/usr/local/bin/wt
worktrunk_config_path=~/.config/worktrunk/config.toml
worktrunk_copy_ignored=true
"#;
        config.parse_content(content, "test".into()).unwrap();
        assert_eq!(config.worktree_provider, WorktreeProvider::Worktrunk);
        assert_eq!(config.worktrunk_bin, PathBuf::from("/usr/local/bin/wt"));
        assert_eq!(
            config.worktrunk_config_path,
            Some(PathBuf::from("~/.config/worktrunk/config.toml"))
        );
        assert!(config.worktrunk_copy_ignored);
    }

    #[test]
    fn parse_worktree_provider_git() {
        let mut config = Config::default();
        let content = "worktree_provider=git";
        config.parse_content(content, "test".into()).unwrap();
        assert_eq!(config.worktree_provider, WorktreeProvider::Git);
    }

    #[test]
    fn parse_worktree_provider_invalid() {
        let mut config = Config::default();
        let content = "worktree_provider=invalid";
        let result = config.parse_content(content, "test".into());
        assert!(result.is_err());
    }

    #[test]
    fn default_config_has_expected_postmortem_values() {
        let config = Config::default();
        assert!(config.summary_json);
        assert!(config.postmortem);
    }

    #[test]
    fn parse_postmortem_config() {
        let mut config = Config::default();
        let content = r#"
summary_json=false
postmortem=false
"#;
        config.parse_content(content, "test".into()).unwrap();
        assert!(!config.summary_json);
        assert!(!config.postmortem);
    }

    #[test]
    fn default_config_has_expected_skills_values() {
        let config = Config::default();
        assert!(!config.skills_enabled);
        assert_eq!(config.skills_builtin_dir, PathBuf::from("skills"));
        assert!(config.skills_sync_on_start);
        assert!(!config.skills_dirs.is_empty());
        assert_eq!(config.skills_max_selected_impl, 2);
        assert_eq!(config.skills_max_selected_review, 1);
        assert!(!config.skills_load_references);
        assert_eq!(config.skills_max_body_chars, 20000);
    }

    #[test]
    fn parse_skills_config() {
        let mut config = Config::default();
        let content = r#"
skills_enabled=true
skills_builtin_dir=/opt/skills
skills_sync_dir=/var/lib/loopd/skills
skills_sync_on_start=false
skills_dirs=.skills ~/.skills
skills_max_selected_impl=3
skills_max_selected_review=2
skills_load_references=true
skills_max_body_chars=50000
"#;
        config.parse_content(content, "test".into()).unwrap();
        assert!(config.skills_enabled);
        assert_eq!(config.skills_builtin_dir, PathBuf::from("/opt/skills"));
        assert_eq!(config.skills_sync_dir, PathBuf::from("/var/lib/loopd/skills"));
        assert!(!config.skills_sync_on_start);
        assert_eq!(config.skills_dirs, vec![PathBuf::from(".skills"), PathBuf::from("~/.skills")]);
        assert_eq!(config.skills_max_selected_impl, 3);
        assert_eq!(config.skills_max_selected_review, 2);
        assert!(config.skills_load_references);
        assert_eq!(config.skills_max_body_chars, 50000);
    }
}
