use crate::config::InheritDecision;
use std::collections::HashSet;

/// User-level subdirs of `~/.claude` inherited into each account's config dir,
/// in the order they are processed.
#[allow(dead_code)]
pub const INHERITED_SUBDIRS: &[&str] =
    &["agents", "commands", "skills", "output-styles", "plugins"];

/// One entry observed in an account's subdir during planning.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)]
pub struct DestEntry {
    pub name: String,
    /// True if the entry is a symlink (created by us); false = account-owned.
    pub is_symlink: bool,
}

/// A single symlink to create, by entry name.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)]
pub struct LinkAction {
    pub name: String,
}

/// The resolved action for one subdir.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)]
pub enum SubdirPlan {
    /// Ensure the subdir exists and create these links (may be empty = no-op).
    Link(Vec<LinkAction>),
    /// Leave the subdir untouched.
    Skip,
    /// Conflict with no usable decision — the caller must prompt the user.
    NeedsPrompt,
}

/// Names to link: every source entry whose name is absent from the dest.
/// Sorted ascending for deterministic behavior and tests.
#[allow(dead_code)]
pub fn plan_links(source_entries: &[String], dest_entries: &[DestEntry]) -> Vec<LinkAction> {
    let existing: HashSet<&str> = dest_entries.iter().map(|e| e.name.as_str()).collect();
    let mut out: Vec<LinkAction> = source_entries
        .iter()
        .filter(|n| !existing.contains(n.as_str()))
        .map(|n| LinkAction { name: n.clone() })
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// True if the dest has any real (non-symlink) entry — i.e. account-owned content.
#[allow(dead_code)]
pub fn has_conflict(dest_entries: &[DestEntry]) -> bool {
    dest_entries.iter().any(|e| !e.is_symlink)
}

/// Resolve what to do for one subdir from its persisted decision and current state.
#[allow(dead_code)]
pub fn resolve_subdir(
    decision: Option<&InheritDecision>,
    source_entries: &[String],
    dest_entries: &[DestEntry],
) -> SubdirPlan {
    match decision {
        Some(InheritDecision::Merge) => {
            SubdirPlan::Link(plan_links(source_entries, dest_entries))
        }
        Some(InheritDecision::Skip) => {
            // Stale check: honor skip only while the account still has own entries.
            if has_conflict(dest_entries) {
                SubdirPlan::Skip
            } else {
                SubdirPlan::NeedsPrompt
            }
        }
        None => {
            if has_conflict(dest_entries) {
                SubdirPlan::NeedsPrompt
            } else {
                SubdirPlan::Link(plan_links(source_entries, dest_entries))
            }
        }
    }
}

#[cfg(test)]
mod core_tests {
    use super::*;

    fn dest(name: &str, is_symlink: bool) -> DestEntry {
        DestEntry { name: name.to_string(), is_symlink }
    }

    #[test]
    fn test_should_link_all_source_entries_when_dest_empty() {
        let src = vec!["a.md".to_string(), "b.md".to_string()];
        let actions = plan_links(&src, &[]);
        let names: Vec<_> = actions.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, vec!["a.md", "b.md"]);
    }

    #[test]
    fn test_should_skip_colliding_names_when_planning_links() {
        let src = vec!["a.md".to_string(), "b.md".to_string()];
        let dst = vec![dest("a.md", false)];
        let actions = plan_links(&src, &dst);
        let names: Vec<_> = actions.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, vec!["b.md"]); // a.md is account-owned, kept
    }

    #[test]
    fn test_should_report_conflict_when_dest_has_real_entry() {
        assert!(has_conflict(&[dest("x.md", false)]));
    }

    #[test]
    fn test_should_not_report_conflict_when_dest_only_symlinks() {
        assert!(!has_conflict(&[dest("x.md", true), dest("y.md", true)]));
    }

    #[test]
    fn test_should_plan_links_when_no_decision_and_no_conflict() {
        let src = vec!["a.md".to_string()];
        let plan = resolve_subdir(None, &src, &[dest("a.md", true)]);
        assert_eq!(plan, SubdirPlan::Link(vec![])); // already linked, nothing to do
    }

    #[test]
    fn test_should_need_prompt_when_no_decision_and_conflict() {
        let src = vec!["a.md".to_string()];
        let plan = resolve_subdir(None, &src, &[dest("own.md", false)]);
        assert_eq!(plan, SubdirPlan::NeedsPrompt);
    }

    #[test]
    fn test_should_link_when_merge_decision_with_conflict() {
        let src = vec!["a.md".to_string(), "own.md".to_string()];
        let plan = resolve_subdir(
            Some(&InheritDecision::Merge),
            &src,
            &[dest("own.md", false)],
        );
        assert_eq!(plan, SubdirPlan::Link(vec![LinkAction { name: "a.md".into() }]));
    }

    #[test]
    fn test_should_skip_when_skip_decision_and_own_entries_present() {
        let src = vec!["a.md".to_string()];
        let plan = resolve_subdir(
            Some(&InheritDecision::Skip),
            &src,
            &[dest("own.md", false)],
        );
        assert_eq!(plan, SubdirPlan::Skip);
    }

    #[test]
    fn test_should_reprompt_when_skip_decision_is_stale() {
        // Skip persisted but no own entries left → stale → re-prompt.
        let src = vec!["a.md".to_string()];
        let plan = resolve_subdir(Some(&InheritDecision::Skip), &src, &[]);
        assert_eq!(plan, SubdirPlan::NeedsPrompt);
    }
}
