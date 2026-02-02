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
    /// Skill hints extracted from the task text (e.g., `@skill-name` mentions).
    pub skill_hints: Vec<String>,
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
            let skill_hints = extract_skill_hints(task_text);
            return Some(TaskSelection {
                label: task_text.to_string(),
                line_number: line_idx + 1,
                section: current_section,
                skill_hints,
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

/// Extracts skill hints from task text.
///
/// Skill hints use `@skill-name` syntax. Valid skill names per spec §3:
/// - 1-64 chars
/// - Lowercase letters, numbers, hyphens
/// - No leading/trailing hyphen
/// - No consecutive hyphens
///
/// Returns a list of unique skill names in order of appearance.
pub fn extract_skill_hints(text: &str) -> Vec<String> {
    let mut hints = Vec::new();
    let mut seen = std::collections::HashSet::new();

    // Match @skill-name patterns.
    // Start after @ and capture valid skill name characters.
    let mut chars = text.char_indices().peekable();

    while let Some((idx, ch)) = chars.next() {
        if ch == '@' {
            // Extract the skill name following the @.
            let start = idx + 1;
            let mut end = start;

            // Collect valid skill name characters: [a-z0-9-]
            while let Some(&(i, c)) = chars.peek() {
                if c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' {
                    end = i + c.len_utf8();
                    chars.next();
                } else {
                    break;
                }
            }

            if end > start {
                let name = &text[start..end];
                // Validate skill name constraints:
                // - No leading/trailing hyphen
                // - No consecutive hyphens
                // - Length 1-64
                if is_valid_skill_name(name) && seen.insert(name.to_string()) {
                    hints.push(name.to_string());
                }
            }
        }
    }

    hints
}

/// Validates a skill name per spec §3 constraints.
fn is_valid_skill_name(name: &str) -> bool {
    if name.is_empty() || name.len() > 64 {
        return false;
    }
    if name.starts_with('-') || name.ends_with('-') {
        return false;
    }
    if name.contains("--") {
        return false;
    }
    // Must contain only lowercase letters, numbers, hyphens.
    name.chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
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

    // Skill hint parsing tests.

    #[test]
    fn extracts_single_skill_hint() {
        let content = "- [ ] Implement feature @my-skill";
        let selection = select_task_from_content(content).unwrap();
        assert_eq!(selection.skill_hints, vec!["my-skill"]);
    }

    #[test]
    fn extracts_multiple_skill_hints() {
        let content = "- [ ] Task @skill1 and @skill2 together";
        let selection = select_task_from_content(content).unwrap();
        assert_eq!(selection.skill_hints, vec!["skill1", "skill2"]);
    }

    #[test]
    fn dedupes_skill_hints() {
        let content = "- [ ] Task @skill1 then @skill1 again";
        let selection = select_task_from_content(content).unwrap();
        assert_eq!(selection.skill_hints, vec!["skill1"]);
    }

    #[test]
    fn skill_hints_empty_when_none() {
        let content = "- [ ] Task with no skill hints";
        let selection = select_task_from_content(content).unwrap();
        assert!(selection.skill_hints.is_empty());
    }

    #[test]
    fn rejects_invalid_skill_names() {
        // Leading hyphen.
        assert!(extract_skill_hints("@-invalid").is_empty());
        // Trailing hyphen.
        assert!(extract_skill_hints("@invalid-").is_empty());
        // Consecutive hyphens.
        assert!(extract_skill_hints("@in--valid").is_empty());
        // Uppercase letters.
        assert!(extract_skill_hints("@Invalid").is_empty());
        // Empty name.
        assert!(extract_skill_hints("@ no-name").is_empty());
    }

    #[test]
    fn accepts_valid_skill_names() {
        assert_eq!(extract_skill_hints("@a"), vec!["a"]);
        assert_eq!(extract_skill_hints("@skill-name"), vec!["skill-name"]);
        assert_eq!(extract_skill_hints("@skill123"), vec!["skill123"]);
        assert_eq!(extract_skill_hints("@a-b-c"), vec!["a-b-c"]);
    }

    #[test]
    fn skill_hint_at_end_of_text() {
        let hints = extract_skill_hints("Implement feature @my-skill");
        assert_eq!(hints, vec!["my-skill"]);
    }

    #[test]
    fn skill_hint_with_punctuation_after() {
        let hints = extract_skill_hints("Use @my-skill, then @another-skill.");
        assert_eq!(hints, vec!["my-skill", "another-skill"]);
    }

    #[test]
    fn skill_hint_in_parentheses() {
        let hints = extract_skill_hints("(@my-skill)");
        assert_eq!(hints, vec!["my-skill"]);
    }

    #[test]
    fn valid_skill_name_validation() {
        assert!(is_valid_skill_name("a"));
        assert!(is_valid_skill_name("skill"));
        assert!(is_valid_skill_name("my-skill"));
        assert!(is_valid_skill_name("skill123"));
        assert!(is_valid_skill_name("a1b2c3"));

        assert!(!is_valid_skill_name(""));
        assert!(!is_valid_skill_name("-skill"));
        assert!(!is_valid_skill_name("skill-"));
        assert!(!is_valid_skill_name("sk--ill"));
        assert!(!is_valid_skill_name("Skill"));
        assert!(!is_valid_skill_name("skill_name"));
        // 65 characters - too long.
        assert!(!is_valid_skill_name(
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        ));
    }

    #[test]
    fn skill_64_char_name_valid() {
        // Exactly 64 characters - should be valid.
        let name = "a".repeat(64);
        assert!(is_valid_skill_name(&name));
    }
}
