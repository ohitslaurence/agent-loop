//! Plan task parsing and selection utilities.
//!
//! Implements the plan task selector from the Open Skills orchestration spec.
//! Parses plan markdown files with checkbox syntax and selects the next unchecked task.

use std::fs;
use std::path::Path;

/// Result of selecting a task from a plan file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskSelection {
    /// The full text of the selected task (checkbox line content after the checkbox).
    pub label: String,
    /// Line number in the plan file (1-indexed).
    pub line_number: usize,
    /// The section heading this task appears under, if any.
    pub section: Option<String>,
}

/// Error type for plan parsing.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PlanError {
    #[error("failed to read plan file: {0}")]
    IoError(String),
    #[error("no unchecked tasks found in plan")]
    NoTasks,
}

/// Parses a plan file and selects the next unchecked task.
///
/// Per spec Section 5.1:
/// - Selects the first `- [ ]` task that is not in a verification checklist section
/// - Ignores `- [ ]?` items (manual QA only)
/// - Ignores tasks inside code blocks
///
/// Per spec Section 6.1:
/// - Returns `None` if the plan cannot be parsed (fallback to agent-chosen task)
pub fn select_task(plan_path: &Path) -> Result<Option<TaskSelection>, PlanError> {
    let content = fs::read_to_string(plan_path)
        .map_err(|e| PlanError::IoError(e.to_string()))?;

    Ok(select_task_from_content(&content))
}

/// Parses plan content and selects the next unchecked task.
///
/// This is the core parsing logic, separated for testing.
pub fn select_task_from_content(content: &str) -> Option<TaskSelection> {
    let mut in_code_block = false;
    let mut in_verification_section = false;
    let mut current_section: Option<String> = None;

    for (line_idx, line) in content.lines().enumerate() {
        let trimmed = line.trim();

        // Track code blocks (toggle on ``` lines).
        if trimmed.starts_with("```") {
            in_code_block = !in_code_block;
            continue;
        }

        // Skip content inside code blocks.
        if in_code_block {
            continue;
        }

        // Track section headings.
        if trimmed.starts_with("## ") {
            let heading = trimmed[3..].trim();
            // Check if this is a verification/QA section.
            let heading_lower = heading.to_lowercase();
            in_verification_section = heading_lower.contains("verification")
                || heading_lower.starts_with("manual qa")
                || heading_lower.contains("checklist");
            current_section = Some(heading.to_string());
            continue;
        }

        // Skip tasks in verification sections.
        if in_verification_section {
            continue;
        }

        // Look for unchecked task checkboxes: `- [ ]` not followed by `?`.
        // Must match: `- [ ] task text` (with space after checkbox).
        // Must NOT match: `- [ ]?` (manual QA marker).
        if let Some(task_text) = parse_unchecked_task(trimmed) {
            return Some(TaskSelection {
                label: task_text.to_string(),
                line_number: line_idx + 1,
                section: current_section,
            });
        }
    }

    None
}

/// Parses an unchecked task checkbox line.
///
/// Returns the task text if the line matches `- [ ] <task>` (not `- [ ]?`).
fn parse_unchecked_task(line: &str) -> Option<&str> {
    // Match lines starting with `- [ ]` (with optional leading whitespace stripped).
    let after_checkbox = line.strip_prefix("- [ ]")?;

    // Reject `- [ ]?` (manual QA marker).
    if after_checkbox.starts_with('?') {
        return None;
    }

    // Must have a space after checkbox (or be empty/whitespace-only for edge case).
    if after_checkbox.is_empty() {
        return None;
    }

    if !after_checkbox.starts_with(' ') {
        return None;
    }

    // Return the task text after the space.
    let task_text = after_checkbox[1..].trim();
    if task_text.is_empty() {
        return None;
    }

    Some(task_text)
}

/// Counts unchecked tasks in a plan file (excluding verification sections and `[ ]?` items).
pub fn count_pending_tasks(content: &str) -> usize {
    let mut count = 0;
    let mut in_code_block = false;
    let mut in_verification_section = false;

    for line in content.lines() {
        let trimmed = line.trim();

        if trimmed.starts_with("```") {
            in_code_block = !in_code_block;
            continue;
        }

        if in_code_block {
            continue;
        }

        if trimmed.starts_with("## ") {
            let heading = trimmed[3..].trim().to_lowercase();
            in_verification_section = heading.contains("verification")
                || heading.starts_with("manual qa")
                || heading.contains("checklist");
            continue;
        }

        if in_verification_section {
            continue;
        }

        if parse_unchecked_task(trimmed).is_some() {
            count += 1;
        }
    }

    count
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selects_first_unchecked_task() {
        let content = r#"
# Plan

## Phase 1
- [x] Completed task
- [ ] First unchecked task
- [ ] Second unchecked task
"#;
        let selection = select_task_from_content(content).unwrap();
        assert_eq!(selection.label, "First unchecked task");
        assert_eq!(selection.line_number, 6);
        assert_eq!(selection.section, Some("Phase 1".to_string()));
    }

    #[test]
    fn skips_verification_section() {
        let content = r#"
## Implementation
- [ ] Implement feature

## Verification Checklist
- [ ] Run tests
- [ ] Check coverage
"#;
        let selection = select_task_from_content(content).unwrap();
        assert_eq!(selection.label, "Implement feature");
        assert_eq!(selection.section, Some("Implementation".to_string()));
    }

    #[test]
    fn skips_manual_qa_items() {
        let content = r#"
## Tasks
- [ ]? Manual QA item (ignored)
- [ ] Actual task
"#;
        let selection = select_task_from_content(content).unwrap();
        assert_eq!(selection.label, "Actual task");
    }

    #[test]
    fn skips_code_blocks() {
        let content = r#"
## Tasks
```markdown
- [ ] Task inside code block (ignored)
```
- [ ] Task outside code block
"#;
        let selection = select_task_from_content(content).unwrap();
        assert_eq!(selection.label, "Task outside code block");
    }

    #[test]
    fn returns_none_when_all_complete() {
        let content = r#"
## Tasks
- [x] Completed task
- [R] Reviewed task
"#;
        assert!(select_task_from_content(content).is_none());
    }

    #[test]
    fn returns_none_for_empty_plan() {
        assert!(select_task_from_content("").is_none());
        assert!(select_task_from_content("# Empty Plan\n\nNo tasks.").is_none());
    }

    #[test]
    fn handles_blocked_tasks() {
        // Blocked tasks [~] should NOT be selected.
        let content = r#"
## Tasks
- [~] Blocked task
- [ ] Unchecked task
"#;
        let selection = select_task_from_content(content).unwrap();
        assert_eq!(selection.label, "Unchecked task");
    }

    #[test]
    fn handles_various_checkbox_states() {
        let content = r#"
## Tasks
- [x] Completed
- [~] Blocked
- [R] Reviewed
- [ ] Pending
"#;
        let selection = select_task_from_content(content).unwrap();
        assert_eq!(selection.label, "Pending");
    }

    #[test]
    fn skips_verification_heading_variants() {
        // Test various verification section heading patterns.
        let content = r#"
## Implementation Tasks
- [ ] Task 1

## Verification
- [ ] Should be skipped

## Manual QA Checklist
- [ ] Also skipped

## More Tasks
- [ ] Task 2
"#;
        let selection = select_task_from_content(content).unwrap();
        assert_eq!(selection.label, "Task 1");
    }

    #[test]
    fn counts_pending_tasks() {
        let content = r#"
## Phase 1
- [x] Done
- [ ] Pending 1
- [ ] Pending 2

## Verification
- [ ] Skipped
"#;
        assert_eq!(count_pending_tasks(content), 2);
    }

    #[test]
    fn count_excludes_manual_qa() {
        let content = r#"
## Tasks
- [ ] Real task
- [ ]? Manual QA
"#;
        assert_eq!(count_pending_tasks(content), 1);
    }

    #[test]
    fn rejects_malformed_checkboxes() {
        // These should not be parsed as tasks.
        let content = r#"
- [] Missing space in checkbox
- [ ]No space after checkbox
- [x] Completed
-[ ] No space before checkbox
"#;
        assert!(select_task_from_content(content).is_none());
    }

    #[test]
    fn handles_nested_code_blocks() {
        let content = r#"
```rust
// Some code with ``backticks``
```
- [ ] Task after code
"#;
        let selection = select_task_from_content(content).unwrap();
        assert_eq!(selection.label, "Task after code");
    }

    #[test]
    fn tracks_section_across_tasks() {
        let content = r#"
## Section A
- [x] Done

## Section B
- [ ] Task in B
"#;
        let selection = select_task_from_content(content).unwrap();
        assert_eq!(selection.section, Some("Section B".to_string()));
    }

    #[test]
    fn handles_whitespace_in_task_text() {
        let content = "- [ ]   Task with extra spaces   ";
        let selection = select_task_from_content(content).unwrap();
        assert_eq!(selection.label, "Task with extra spaces");
    }

    #[test]
    fn checklist_heading_detection() {
        // "Checklist" in heading should mark as verification section.
        let content = r#"
## Implementation Checklist
- [ ] Skipped (checklist)

## Regular Tasks
- [ ] Real task
"#;
        let selection = select_task_from_content(content).unwrap();
        assert_eq!(selection.label, "Real task");
    }
}
