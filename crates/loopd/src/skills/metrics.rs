//! Skills metrics collection.
//!
//! Implements spec Section 7.2: metrics for skill discovery, selection, and failures.
//!
//! Uses atomic counters following the scheduler pattern for lightweight metrics.

use std::sync::atomic::{AtomicUsize, Ordering};

/// Metrics for skill operations.
///
/// Per open-skills-orchestration.md Section 7.2.
#[derive(Debug, Default)]
pub struct SkillsMetrics {
    /// Total number of skills discovered across all runs.
    pub discovered_total: AtomicUsize,
    /// Total number of skills selected across all runs.
    pub selected_total: AtomicUsize,
    /// Total number of skill load failures.
    pub load_failed_total: AtomicUsize,
    /// Total number of skill truncations.
    pub truncated_total: AtomicUsize,
}

impl SkillsMetrics {
    /// Create a new metrics instance with all counters at zero.
    pub fn new() -> Self {
        Self::default()
    }

    /// Increment the discovered counter by the given count.
    pub fn inc_discovered(&self, count: usize) {
        self.discovered_total.fetch_add(count, Ordering::Relaxed);
    }

    /// Increment the selected counter by the given count.
    pub fn inc_selected(&self, count: usize) {
        self.selected_total.fetch_add(count, Ordering::Relaxed);
    }

    /// Increment the load_failed counter.
    pub fn inc_load_failed(&self) {
        self.load_failed_total.fetch_add(1, Ordering::Relaxed);
    }

    /// Increment the truncated counter.
    pub fn inc_truncated(&self) {
        self.truncated_total.fetch_add(1, Ordering::Relaxed);
    }

    /// Get current discovered count.
    pub fn get_discovered(&self) -> usize {
        self.discovered_total.load(Ordering::Relaxed)
    }

    /// Get current selected count.
    pub fn get_selected(&self) -> usize {
        self.selected_total.load(Ordering::Relaxed)
    }

    /// Get current load_failed count.
    pub fn get_load_failed(&self) -> usize {
        self.load_failed_total.load(Ordering::Relaxed)
    }

    /// Get current truncated count.
    pub fn get_truncated(&self) -> usize {
        self.truncated_total.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn increments_discovered() {
        let metrics = SkillsMetrics::new();
        assert_eq!(metrics.get_discovered(), 0);
        metrics.inc_discovered(5);
        assert_eq!(metrics.get_discovered(), 5);
        metrics.inc_discovered(3);
        assert_eq!(metrics.get_discovered(), 8);
    }

    #[test]
    fn increments_selected() {
        let metrics = SkillsMetrics::new();
        assert_eq!(metrics.get_selected(), 0);
        metrics.inc_selected(2);
        assert_eq!(metrics.get_selected(), 2);
    }

    #[test]
    fn increments_load_failed() {
        let metrics = SkillsMetrics::new();
        assert_eq!(metrics.get_load_failed(), 0);
        metrics.inc_load_failed();
        metrics.inc_load_failed();
        assert_eq!(metrics.get_load_failed(), 2);
    }

    #[test]
    fn increments_truncated() {
        let metrics = SkillsMetrics::new();
        assert_eq!(metrics.get_truncated(), 0);
        metrics.inc_truncated();
        assert_eq!(metrics.get_truncated(), 1);
    }
}
