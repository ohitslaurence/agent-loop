//! Skill matching and selection.
//!
//! Implements spec Section 4.2 and 5.1: select skills for a task based on hints
//! and keyword matching, respecting per-step limits.

use loop_core::plan::TaskSelection;
use loop_core::skills::SkillMetadata;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Step kind for skill selection limits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepKind {
    Implementation,
    Review,
}

impl StepKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Implementation => "implementation",
            Self::Review => "review",
        }
    }
}

/// Selection strategy used for choosing skills.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SelectionStrategy {
    /// Skills were selected based on explicit `@skill-name` hints in task text.
    Hint,
    /// Skills were selected based on keyword matching.
    Match,
    /// No skills were selected.
    None,
}

/// A selected skill with selection reason.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelectedSkill {
    /// Skill name.
    pub name: String,
    /// Reason for selection (e.g., "hint: @skill-name" or "keyword: pdf").
    pub reason: String,
}

/// Result of skill selection for a task.
///
/// Matches spec Section 3.1 SkillSelection data model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillSelection {
    /// Run ID for event correlation.
    pub run_id: Uuid,
    /// Step kind (implementation or review).
    pub step_kind: StepKind,
    /// Task label from the plan.
    pub task_label: String,
    /// Selected skills with reasons.
    pub skills: Vec<SelectedSkill>,
    /// Selection strategy used.
    pub strategy: SelectionStrategy,
    /// Errors encountered during selection (e.g., hinted skill not found).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<String>,
}

/// Selects skills for a task based on hints and keyword matching.
///
/// Per spec Section 5.1:
/// 1. If the task includes skill hints (@skill-name), select those first.
/// 2. Fill remaining slots with keyword matches from skill name/description.
/// 3. Respect the per-step limit (implementation vs review).
///
/// Per spec Section 5.2:
/// - If a hinted skill is not found, record an error and continue.
pub fn select_skills(
    run_id: Uuid,
    task: &TaskSelection,
    available_skills: &[SkillMetadata],
    step_kind: StepKind,
    max_skills: u8,
) -> SkillSelection {
    let max_skills = max_skills as usize;
    let mut selected: Vec<SelectedSkill> = Vec::new();
    let mut errors: Vec<String> = Vec::new();
    let mut used_names: std::collections::HashSet<&str> = std::collections::HashSet::new();

    // Phase 1: Select hinted skills.
    for hint in &task.skill_hints {
        if selected.len() >= max_skills {
            break;
        }

        if let Some(skill) = available_skills.iter().find(|s| &s.name == hint) {
            if used_names.insert(&skill.name) {
                selected.push(SelectedSkill {
                    name: skill.name.clone(),
                    reason: format!("hint: @{}", hint),
                });
            }
        } else {
            errors.push(format!("hinted skill not found: @{}", hint));
        }
    }

    // Phase 2: Fill remaining slots with keyword matches.
    let remaining_slots = max_skills.saturating_sub(selected.len());
    if remaining_slots > 0 {
        let keyword_matches =
            find_keyword_matches(&task.label, available_skills, &used_names, remaining_slots);
        selected.extend(keyword_matches);
    }

    // Determine strategy.
    let strategy = if selected.is_empty() {
        SelectionStrategy::None
    } else if task.skill_hints.iter().any(|h| {
        selected
            .iter()
            .any(|s| s.reason.contains(&format!("@{}", h)))
    }) {
        SelectionStrategy::Hint
    } else {
        SelectionStrategy::Match
    };

    SkillSelection {
        run_id,
        step_kind,
        task_label: task.label.clone(),
        skills: selected,
        strategy,
        errors,
    }
}

/// Finds keyword matches between task text and skill metadata.
///
/// Scores skills by counting keyword overlaps between the task text and
/// skill name/description. Returns the top N matches.
fn find_keyword_matches<'a>(
    task_text: &str,
    available_skills: &'a [SkillMetadata],
    exclude: &std::collections::HashSet<&str>,
    limit: usize,
) -> Vec<SelectedSkill> {
    if limit == 0 {
        return Vec::new();
    }

    // Extract keywords from task text (lowercase, alphanumeric, min 3 chars).
    let task_keywords: std::collections::HashSet<String> = extract_keywords(task_text);

    if task_keywords.is_empty() {
        return Vec::new();
    }

    // Score each skill.
    let mut scored: Vec<(&'a SkillMetadata, usize, String)> = Vec::new();

    for skill in available_skills {
        if exclude.contains(skill.name.as_str()) {
            continue;
        }

        // Combine name and description for keyword matching.
        // Split name on hyphens to treat "pdf-processing" as ["pdf", "processing"].
        let name_keywords: std::collections::HashSet<String> = skill
            .name
            .split('-')
            .filter(|s| s.len() >= 2)
            .map(|s| s.to_lowercase())
            .collect();

        let desc_keywords = extract_keywords(&skill.description);

        // Find matching keywords.
        let mut matches: Vec<String> = Vec::new();

        for kw in &task_keywords {
            if name_keywords.contains(kw) {
                matches.push(kw.clone());
            } else if desc_keywords.contains(kw) {
                matches.push(kw.clone());
            }
        }

        if !matches.is_empty() {
            // Weight name matches higher than description matches.
            let name_match_count = matches
                .iter()
                .filter(|m| name_keywords.contains(*m))
                .count();
            let score = name_match_count * 2 + matches.len();

            // Build reason string with first few matching keywords.
            let reason_keywords: Vec<_> = matches.iter().take(3).cloned().collect();
            let reason = format!("keyword: {}", reason_keywords.join(", "));

            scored.push((skill, score, reason));
        }
    }

    // Sort by score descending.
    scored.sort_by(|a, b| b.1.cmp(&a.1));

    // Take top matches up to limit.
    scored
        .into_iter()
        .take(limit)
        .map(|(skill, _, reason)| SelectedSkill {
            name: skill.name.clone(),
            reason,
        })
        .collect()
}

/// Extracts keywords from text for matching.
///
/// Returns lowercase words that are at least 3 characters and alphanumeric.
fn extract_keywords(text: &str) -> std::collections::HashSet<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|word| word.len() >= 3)
        .map(|word| word.to_lowercase())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use loop_core::skills::SkillLocation;
    use std::path::PathBuf;

    fn make_skill(name: &str, description: &str) -> SkillMetadata {
        SkillMetadata {
            name: name.to_string(),
            description: description.to_string(),
            license: None,
            compatibility: None,
            metadata: std::collections::HashMap::new(),
            allowed_tools: Vec::new(),
            path: PathBuf::from(format!("/skills/{}", name)),
            location: SkillLocation::Project,
        }
    }

    fn make_task(label: &str, hints: Vec<&str>) -> TaskSelection {
        TaskSelection {
            label: label.to_string(),
            line_number: 1,
            section: None,
            skill_hints: hints.into_iter().map(String::from).collect(),
        }
    }

    #[test]
    fn selects_hinted_skills() {
        let skills = vec![
            make_skill("pdf-processing", "Extract text from PDF files."),
            make_skill("code-review", "Review code for best practices."),
        ];
        let task = make_task(
            "Implement PDF export @pdf-processing",
            vec!["pdf-processing"],
        );
        let run_id = Uuid::nil();

        let selection = select_skills(run_id, &task, &skills, StepKind::Implementation, 2);

        assert_eq!(selection.skills.len(), 1);
        assert_eq!(selection.skills[0].name, "pdf-processing");
        assert!(selection.skills[0].reason.contains("@pdf-processing"));
        assert_eq!(selection.strategy, SelectionStrategy::Hint);
        assert!(selection.errors.is_empty());
    }

    #[test]
    fn records_error_for_missing_hint() {
        let skills = vec![make_skill("code-review", "Review code for best practices.")];
        let task = make_task("Task @nonexistent-skill", vec!["nonexistent-skill"]);
        let run_id = Uuid::nil();

        let selection = select_skills(run_id, &task, &skills, StepKind::Implementation, 2);

        assert!(selection.skills.is_empty());
        assert_eq!(selection.strategy, SelectionStrategy::None);
        assert_eq!(selection.errors.len(), 1);
        assert!(selection.errors[0].contains("nonexistent-skill"));
    }

    #[test]
    fn fills_remaining_slots_with_keyword_matches() {
        let skills = vec![
            make_skill("pdf-processing", "Extract text from PDF files."),
            make_skill("code-review", "Review code for best practices."),
            make_skill("testing", "Run automated tests."),
        ];
        let task = make_task(
            "Implement PDF export @code-review and extract text",
            vec!["code-review"],
        );
        let run_id = Uuid::nil();

        let selection = select_skills(run_id, &task, &skills, StepKind::Implementation, 2);

        assert_eq!(selection.skills.len(), 2);
        assert_eq!(selection.skills[0].name, "code-review");
        assert_eq!(selection.skills[1].name, "pdf-processing");
        assert_eq!(selection.strategy, SelectionStrategy::Hint);
    }

    #[test]
    fn respects_max_skills_limit() {
        let skills = vec![
            make_skill("skill1", "Description for skill one."),
            make_skill("skill2", "Description for skill two."),
            make_skill("skill3", "Description for skill three."),
        ];
        let task = make_task(
            "Task @skill1 @skill2 @skill3",
            vec!["skill1", "skill2", "skill3"],
        );
        let run_id = Uuid::nil();

        let selection = select_skills(run_id, &task, &skills, StepKind::Implementation, 2);

        assert_eq!(selection.skills.len(), 2);
        assert_eq!(selection.skills[0].name, "skill1");
        assert_eq!(selection.skills[1].name, "skill2");
    }

    #[test]
    fn keyword_matching_when_no_hints() {
        let skills = vec![
            make_skill("pdf-processing", "Extract text from PDF files."),
            make_skill("code-review", "Review code for best practices."),
        ];
        let task = make_task("Extract text from the PDF document", vec![]);
        let run_id = Uuid::nil();

        let selection = select_skills(run_id, &task, &skills, StepKind::Implementation, 2);

        assert_eq!(selection.skills.len(), 1);
        assert_eq!(selection.skills[0].name, "pdf-processing");
        assert!(selection.skills[0].reason.contains("keyword"));
        assert_eq!(selection.strategy, SelectionStrategy::Match);
    }

    #[test]
    fn no_skills_selected_when_no_matches() {
        let skills = vec![make_skill("unrelated", "Does something unrelated.")];
        let task = make_task("Implement the database migration", vec![]);
        let run_id = Uuid::nil();

        let selection = select_skills(run_id, &task, &skills, StepKind::Implementation, 2);

        assert!(selection.skills.is_empty());
        assert_eq!(selection.strategy, SelectionStrategy::None);
    }

    #[test]
    fn avoids_duplicate_selection() {
        let skills = vec![make_skill("pdf-processing", "Extract text from PDF files.")];
        // Hint and keyword would both match pdf-processing.
        let task = make_task("Process PDF files @pdf-processing", vec!["pdf-processing"]);
        let run_id = Uuid::nil();

        let selection = select_skills(run_id, &task, &skills, StepKind::Implementation, 2);

        assert_eq!(selection.skills.len(), 1);
        assert_eq!(selection.skills[0].name, "pdf-processing");
    }

    #[test]
    fn step_kind_affects_serialization() {
        let selection = SkillSelection {
            run_id: Uuid::nil(),
            step_kind: StepKind::Review,
            task_label: "Test".to_string(),
            skills: Vec::new(),
            strategy: SelectionStrategy::None,
            errors: Vec::new(),
        };

        let json = serde_json::to_string(&selection).unwrap();
        assert!(json.contains("\"step_kind\":\"review\""));
    }

    #[test]
    fn keyword_matching_prefers_name_over_description() {
        let skills = vec![
            make_skill("database-migration", "Run database migrations."),
            make_skill("unrelated", "This skill handles database operations."),
        ];
        let task = make_task("Implement the database migration", vec![]);
        let run_id = Uuid::nil();

        let selection = select_skills(run_id, &task, &skills, StepKind::Implementation, 1);

        // database-migration should be preferred because "database" and "migration"
        // are both in the name.
        assert_eq!(selection.skills.len(), 1);
        assert_eq!(selection.skills[0].name, "database-migration");
    }

    #[test]
    fn extract_keywords_filters_short_words() {
        let keywords = extract_keywords("A to is PDF test foo");
        assert!(keywords.contains("pdf"));
        assert!(keywords.contains("test"));
        assert!(keywords.contains("foo")); // 3 chars - included
        assert!(!keywords.contains("a")); // 1 char - excluded
        assert!(!keywords.contains("to")); // 2 chars - excluded
        assert!(!keywords.contains("is")); // 2 chars - excluded
    }

    #[test]
    fn serializes_selection_strategy() {
        assert_eq!(
            serde_json::to_string(&SelectionStrategy::Hint).unwrap(),
            "\"hint\""
        );
        assert_eq!(
            serde_json::to_string(&SelectionStrategy::Match).unwrap(),
            "\"match\""
        );
        assert_eq!(
            serde_json::to_string(&SelectionStrategy::None).unwrap(),
            "\"none\""
        );
    }

    #[test]
    fn selection_with_multiple_hints_and_matches() {
        let skills = vec![
            make_skill("pdf-processing", "Extract text from PDF files."),
            make_skill("code-review", "Review code for best practices."),
            make_skill("testing", "Run automated tests."),
            make_skill("formatting", "Format code according to style."),
        ];
        let task = make_task(
            "Review the PDF processing code @code-review and run tests",
            vec!["code-review"],
        );
        let run_id = Uuid::nil();

        let selection = select_skills(run_id, &task, &skills, StepKind::Implementation, 3);

        // Should select code-review (hint), then pdf-processing and testing (keywords).
        assert_eq!(selection.skills.len(), 3);
        assert_eq!(selection.skills[0].name, "code-review");
        // Keyword matches should follow (order may vary by score).
        let keyword_names: Vec<_> = selection.skills[1..]
            .iter()
            .map(|s| s.name.as_str())
            .collect();
        assert!(keyword_names.contains(&"pdf-processing") || keyword_names.contains(&"testing"));
    }

    #[test]
    fn empty_available_skills() {
        let task = make_task("Some task @skill1", vec!["skill1"]);
        let run_id = Uuid::nil();

        let selection = select_skills(run_id, &task, &[], StepKind::Implementation, 2);

        assert!(selection.skills.is_empty());
        assert_eq!(selection.strategy, SelectionStrategy::None);
        assert_eq!(selection.errors.len(), 1);
    }

    #[test]
    fn review_step_kind() {
        let skills = vec![make_skill("code-review", "Review code for best practices.")];
        let task = make_task("Review the implementation", vec![]);
        let run_id = Uuid::nil();

        let selection = select_skills(run_id, &task, &skills, StepKind::Review, 1);

        assert_eq!(selection.step_kind, StepKind::Review);
        assert_eq!(selection.skills.len(), 1);
    }
}
