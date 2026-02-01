//! Skills orchestration support for the daemon.
//!
//! Implements the Open Skills specification for skill discovery, matching, and loading.
//! See: specs/open-skills-orchestration.md

mod catalog;
mod r#match;
mod render;
mod sync;

pub use catalog::{discover_skills, DiscoveryResult};
pub use r#match::{select_skills, SelectedSkill, SelectionStrategy, SkillSelection, StepKind};
pub use render::render_available_skills;
pub use sync::sync_builtin_skills;
