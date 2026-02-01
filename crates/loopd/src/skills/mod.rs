//! Skills orchestration support for the daemon.
//!
//! Implements the Open Skills specification for skill discovery, matching, and loading.
//! See: specs/open-skills-orchestration.md

mod catalog;
mod sync;

pub use catalog::{discover_skills, DiscoveryResult};
pub use sync::sync_builtin_skills;
