//! Skills orchestration support for the daemon.
//!
//! Implements the Open Skills specification for skill discovery, matching, and loading.
//! See: specs/open-skills-orchestration.md

mod catalog;
mod r#match;
mod metrics;
mod render;
mod sync;

pub use catalog::{discover_skills, DiscoveryResult};
pub use r#match::{select_skills, SelectedSkill, SelectionStrategy, SkillSelection, StepKind};
pub use metrics::SkillsMetrics;
pub use render::{load_skill_body, render_available_skills, LoadedSkill, TruncationEvent};
pub use sync::sync_builtin_skills;
