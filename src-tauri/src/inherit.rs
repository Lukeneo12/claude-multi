use crate::config::InheritDecision;
use std::collections::HashSet;
use std::path::Path;

/// User-level subdirs of `~/.claude` inherited into each account's config dir,
/// in the order they are processed.
pub const INHERITED_SUBDIRS: &[&str] =
    &["agents", "commands", "skills", "output-styles", "plugins"];

/// One entry observed in an account's subdir during planning.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DestEntry {
    pub name: String,
    /// True if the entry is a symlink (created by us); false = account-owned.
    pub is_symlink: bool,
}

/// A single symlink to create, by entry name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkAction {
    pub name: String,
}

/// The resolved action for one subdir.
#[derive(Debug, Clone, PartialEq, Eq)]
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
pub fn has_conflict(dest_entries: &[DestEntry]) -> bool {
    dest_entries.iter().any(|e| !e.is_symlink)
}

/// Resolve what to do for one subdir from its persisted decision and current state.
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

/// Result of an inherit pass for one account.
#[allow(dead_code)] // consumed by commands.rs in Task 4
pub struct InheritOutcome {
    /// Subdir names that need a user decision (conflict or stale skip).
    pub needs_prompt: Vec<String>,
}

/// Ensure the account's `config_dir` inherits the shared resources from `source`
/// (e.g. `~/.claude`). For each inherited subdir that exists under `source`,
/// resolves the plan from `decisions` and current dest state and creates links.
/// Subdirs needing a decision are returned in `needs_prompt`; the caller prompts
/// and persists, then calls again. Never writes inside `source`.
#[allow(dead_code)] // consumed by commands.rs in Task 4
pub fn ensure_inherited(
    source: &Path,
    config_dir: &Path,
    decisions: &std::collections::HashMap<String, InheritDecision>,
) -> std::io::Result<InheritOutcome> {
    let mut needs_prompt = Vec::new();
    for sub in INHERITED_SUBDIRS {
        let src_sub = source.join(sub);
        if !src_sub.is_dir() {
            continue; // nothing to inherit from this subdir
        }
        let dest_sub = config_dir.join(sub);
        let source_entries = read_entry_names(&src_sub)?;
        let dest_entries = read_dest_entries(&dest_sub)?;
        match resolve_subdir(decisions.get(*sub), &source_entries, &dest_entries) {
            SubdirPlan::Skip => {}
            SubdirPlan::NeedsPrompt => needs_prompt.push((*sub).to_string()),
            SubdirPlan::Link(actions) => {
                if !actions.is_empty() {
                    std::fs::create_dir_all(&dest_sub)?;
                    for a in actions {
                        create_link(&src_sub.join(&a.name), &dest_sub.join(&a.name))?;
                    }
                }
            }
        }
    }
    Ok(InheritOutcome { needs_prompt })
}

/// List the file names directly under `dir` (non-recursive).
fn read_entry_names(dir: &Path) -> std::io::Result<Vec<String>> {
    let mut names = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        if let Some(name) = entry.file_name().to_str() {
            names.push(name.to_string());
        }
    }
    Ok(names)
}

/// Read dest entries with symlink-ness. Returns empty if the dir doesn't exist.
fn read_dest_entries(dir: &Path) -> std::io::Result<Vec<DestEntry>> {
    let mut entries = Vec::new();
    match std::fs::read_dir(dir) {
        Ok(rd) => {
            for entry in rd {
                let entry = entry?;
                let name = match entry.file_name().to_str() {
                    Some(n) => n.to_string(),
                    None => continue,
                };
                let is_symlink = entry.file_type()?.is_symlink();
                entries.push(DestEntry { name, is_symlink });
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    Ok(entries)
}

/// Create a symlink at `dst` pointing to `src`. On Windows, fall back to a
/// recursive copy if the symlink can't be created (e.g. no privilege).
#[cfg(unix)]
fn create_link(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(src, dst)
}

#[cfg(windows)]
fn create_link(src: &Path, dst: &Path) -> std::io::Result<()> {
    let res = if src.is_dir() {
        std::os::windows::fs::symlink_dir(src, dst)
    } else {
        std::os::windows::fs::symlink_file(src, dst)
    };
    match res {
        Ok(()) => Ok(()),
        Err(_) => copy_recursive(src, dst), // privilege/other failure → copy
    }
}

/// Recursive copy used as the Windows fallback when symlinks are unavailable.
#[cfg(windows)]
fn copy_recursive(src: &Path, dst: &Path) -> std::io::Result<()> {
    if src.is_dir() {
        std::fs::create_dir_all(dst)?;
        for entry in std::fs::read_dir(src)? {
            let entry = entry?;
            copy_recursive(&entry.path(), &dst.join(entry.file_name()))?;
        }
        Ok(())
    } else {
        std::fs::copy(src, dst).map(|_| ())
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

#[cfg(all(test, unix))]
mod io_tests {
    use super::*;
    use crate::config::InheritDecision;
    use std::collections::HashMap;
    use std::os::unix::fs::symlink;

    /// Build a `~/.claude`-like source with the given subdir/entries, plus an
    /// empty account config dir, under a unique tempdir. Returns (source, cfg).
    fn fixture(tag: &str) -> (std::path::PathBuf, std::path::PathBuf) {
        let base = std::env::temp_dir().join(format!("cm_inherit_{tag}"));
        let _ = std::fs::remove_dir_all(&base);
        let source = base.join("dot-claude");
        let cfg = base.join("dot-claude-acct");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::create_dir_all(&cfg).unwrap();
        (source, cfg)
    }

    fn touch(path: &std::path::Path) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, b"x").unwrap();
    }

    #[test]
    fn test_should_link_source_entries_when_dest_subdir_absent() {
        let (source, cfg) = fixture("link_basic");
        touch(&source.join("agents").join("a.md"));
        touch(&source.join("agents").join("b.md"));

        let out = ensure_inherited(&source, &cfg, &HashMap::new()).unwrap();
        assert!(out.needs_prompt.is_empty());

        let linked = cfg.join("agents").join("a.md");
        assert!(std::fs::symlink_metadata(&linked).unwrap().file_type().is_symlink());
        assert_eq!(std::fs::read_link(&linked).unwrap(), source.join("agents").join("a.md"));
        assert!(cfg.join("agents").join("b.md").exists());
    }

    #[test]
    fn test_should_skip_absent_source_subdirs() {
        let (source, cfg) = fixture("absent_src");
        touch(&source.join("commands").join("c.md")); // only commands exists
        let out = ensure_inherited(&source, &cfg, &HashMap::new()).unwrap();
        assert!(out.needs_prompt.is_empty());
        assert!(cfg.join("commands").join("c.md").exists());
        assert!(!cfg.join("agents").exists()); // no agents source → nothing created
    }

    #[test]
    fn test_should_be_idempotent_when_run_twice() {
        let (source, cfg) = fixture("idempotent");
        touch(&source.join("agents").join("a.md"));
        ensure_inherited(&source, &cfg, &HashMap::new()).unwrap();
        // Second run must not error or duplicate.
        let out = ensure_inherited(&source, &cfg, &HashMap::new()).unwrap();
        assert!(out.needs_prompt.is_empty());
        let entries: Vec<_> = std::fs::read_dir(cfg.join("agents"))
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        assert_eq!(entries, vec!["a.md"]);
    }

    #[test]
    fn test_should_need_prompt_when_account_has_own_entries_and_no_decision() {
        let (source, cfg) = fixture("conflict");
        touch(&source.join("agents").join("a.md"));
        touch(&cfg.join("agents").join("own.md")); // account-owned, real file
        let out = ensure_inherited(&source, &cfg, &HashMap::new()).unwrap();
        assert_eq!(out.needs_prompt, vec!["agents".to_string()]);
        // No links created while undecided.
        assert!(!cfg.join("agents").join("a.md").exists());
    }

    #[test]
    fn test_should_merge_when_decision_is_merge() {
        let (source, cfg) = fixture("merge");
        touch(&source.join("agents").join("a.md"));
        touch(&cfg.join("agents").join("own.md"));
        let mut decisions = HashMap::new();
        decisions.insert("agents".to_string(), InheritDecision::Merge);
        let out = ensure_inherited(&source, &cfg, &decisions).unwrap();
        assert!(out.needs_prompt.is_empty());
        assert!(cfg.join("agents").join("a.md").exists());   // inherited
        assert!(cfg.join("agents").join("own.md").exists()); // kept
    }

    #[test]
    fn test_should_skip_when_decision_is_skip_and_own_entries_present() {
        let (source, cfg) = fixture("skip");
        touch(&source.join("agents").join("a.md"));
        touch(&cfg.join("agents").join("own.md"));
        let mut decisions = HashMap::new();
        decisions.insert("agents".to_string(), InheritDecision::Skip);
        let out = ensure_inherited(&source, &cfg, &decisions).unwrap();
        assert!(out.needs_prompt.is_empty());
        assert!(!cfg.join("agents").join("a.md").exists()); // not inherited
    }

    #[test]
    fn test_should_reprompt_when_skip_is_stale() {
        let (source, cfg) = fixture("stale_skip");
        touch(&source.join("agents").join("a.md"));
        std::fs::create_dir_all(cfg.join("agents")).unwrap(); // empty: own entries removed
        let mut decisions = HashMap::new();
        decisions.insert("agents".to_string(), InheritDecision::Skip);
        let out = ensure_inherited(&source, &cfg, &decisions).unwrap();
        assert_eq!(out.needs_prompt, vec!["agents".to_string()]);
    }

    #[test]
    fn test_should_not_treat_existing_symlink_as_conflict() {
        let (source, cfg) = fixture("symlink_no_conflict");
        touch(&source.join("agents").join("a.md"));
        // Pre-existing link as if from an earlier run.
        std::fs::create_dir_all(cfg.join("agents")).unwrap();
        symlink(source.join("agents").join("a.md"), cfg.join("agents").join("a.md")).unwrap();
        let out = ensure_inherited(&source, &cfg, &HashMap::new()).unwrap();
        assert!(out.needs_prompt.is_empty()); // symlink is not account-owned
    }
}
