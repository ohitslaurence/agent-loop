//! Skills orchestration support for the daemon.
//!
//! Implements the Open Skills specification for skill discovery, matching, and loading.
//! See: specs/open-skills-orchestration.md

mod sync;

pub use sync::sync_builtin_skills;
