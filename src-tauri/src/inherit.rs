use crate::config::InheritDecision;
use serde::Serialize;
use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};

/// User-level subdirs of `~/.claude` inherited into each account's config dir,
/// in the order they are processed.
///
/// `plugins` is intentionally excluded: a smoke test showed an inherited
/// `plugins/` loads but reports every plugin as disabled, because plugin
/// enablement lives in the per-account `.claude.json` (never inherited), not in
/// `plugins/`. Symlinking the dir would also expose mutable plugin state
/// (`cache/`, `marketplaces/`, …) to write-through into `~/.claude` on plugin
/// sync. See the spec's Risks section.
pub const INHERITED_SUBDIRS: &[&str] = &["agents", "commands", "skills", "output-styles"];

/// Root-level files of `~/.claude` seeded into an account's config dir when
/// absent there. Unlike `INHERITED_SUBDIRS` these are one-shot real copies,
/// never symlinks: Claude Code writes to them, and a link would write through
/// into `~/.claude`. An existing dest file always wins (no overwrite, no merge).
pub const SEEDED_FILES: &[&str] = &["settings.json"];

/// OS-generated files never inherited (and never counted as account content).
pub const OS_JUNK: &[&str] = &[".DS_Store", "Thumbs.db", "desktop.ini"];

/// True for names in `OS_JUNK`.
/// Case-insensitive: Windows/macOS default filesystems are.
pub fn is_os_junk(name: &str) -> bool {
    OS_JUNK.iter().any(|j| j.eq_ignore_ascii_case(name))
}

/// One entry observed in an account's subdir during planning.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DestEntry {
    pub name: String,
    /// True if the entry is a symlink (created by us); false = account-owned.
    pub is_symlink: bool,
    /// Absolute target of the symlink (relative targets resolved against the
    /// dest subdir); `None` for non-symlinks or an unreadable link.
    pub link_target: Option<PathBuf>,
}

/// A single symlink to create, by entry name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkAction {
    pub name: String,
}

/// Derived, read-only status of one subdir for the Inheritance panel. Unlike
/// `SubdirPlan` (an action), this is a UI-facing classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum InheritStatus {
    /// Shared entries are (or will be) linked in: explicit Merge, or no decision
    /// and no conflict.
    Inherited,
    /// An explicit Skip decision is in effect; nothing is inherited.
    Skipped,
    /// The account has its own entries and no decision is recorded yet.
    Conflict,
    /// Nothing to inherit from `~/.claude` for this subdir.
    None,
}

/// Classify one subdir for the Inheritance panel from its persisted decision and
/// current source/dest state. Pure; mirrors `resolve_subdir`'s semantics.
pub fn subdir_status(
    decision: Option<&InheritDecision>,
    source_entries: &[String],
    dest_entries: &[DestEntry],
) -> InheritStatus {
    if source_entries.iter().all(|n| is_os_junk(n)) {
        return InheritStatus::None;
    }
    match decision {
        Some(InheritDecision::Skip) => InheritStatus::Skipped,
        Some(InheritDecision::Merge) => InheritStatus::Inherited,
        None if has_conflict(dest_entries) => InheritStatus::Conflict,
        None => InheritStatus::Inherited,
    }
}

/// One row of the Inheritance panel: a subdir, its derived status, and the
/// decision currently recorded for it (if any).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InheritSubdirStatus {
    pub subdir: String,
    pub status: InheritStatus,
    pub decision: Option<InheritDecision>,
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
        .filter(|n| !is_os_junk(n) && !existing.contains(n.as_str()))
        .map(|n| LinkAction { name: n.clone() })
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// Name of the `src_sub` entry a link target points at, or `None` when the
/// target is not (lexically) inside `src_sub`. Targets with `..` components are
/// rejected so `<src_sub>/../x` can never be mistaken for an inherited link.
fn inherited_source_name<'a>(target: &'a Path, src_sub: &Path) -> Option<&'a str> {
    let rel = target.strip_prefix(src_sub).ok()?;
    if rel.components().any(|c| !matches!(c, Component::Normal(_))) {
        return None;
    }
    match rel.components().next()? {
        Component::Normal(name) => name.to_str(),
        _ => None,
    }
}

/// Names of dest entries to remove because they are stale inherited links.
///
/// An entry is ours to remove ONLY if it is a symlink whose target lies inside
/// `src_sub` (the `~/.claude/<sub>` we link from). Regular files/dirs
/// (account-owned) and symlinks pointing elsewhere are never listed. With
/// `prune_all` (Skip) every such link goes; otherwise (Merge) only links whose
/// source entry is no longer in `source_entries` (deleted upstream).
///
/// Limitation: the Windows copy fallback produces real copies, which cannot be
/// told apart from account-owned entries, so they are never pruned.
pub fn plan_prune(
    src_sub: &Path,
    source_entries: &[String],
    dest_entries: &[DestEntry],
    prune_all: bool,
) -> Vec<String> {
    // Junk is never planned, so existing junk links count as stale.
    let planned: HashSet<&str> = source_entries
        .iter()
        .map(String::as_str)
        .filter(|n| !is_os_junk(n))
        .collect();
    let mut out: Vec<String> = dest_entries
        .iter()
        .filter(|e| e.is_symlink)
        .filter_map(|e| {
            let name = inherited_source_name(e.link_target.as_deref()?, src_sub)?;
            (prune_all || !planned.contains(name)).then(|| e.name.clone())
        })
        .collect();
    out.sort();
    out
}

/// True if the dest has any real (non-symlink) entry — i.e. account-owned
/// content. OS junk (e.g. a Finder-made `.DS_Store`) is not account content.
pub fn has_conflict(dest_entries: &[DestEntry]) -> bool {
    dest_entries
        .iter()
        .any(|e| !e.is_symlink && !is_os_junk(&e.name))
}

/// Resolve what to do for one subdir from its persisted decision and current state.
pub fn resolve_subdir(
    decision: Option<&InheritDecision>,
    source_entries: &[String],
    dest_entries: &[DestEntry],
) -> SubdirPlan {
    match decision {
        Some(InheritDecision::Merge) => SubdirPlan::Link(plan_links(source_entries, dest_entries)),
        // Sticky decision: a persisted Skip is always honored, even with no
        // conflict left. The user manages it from the Inheritance panel, so we
        // no longer re-prompt when the account's own entries disappear.
        Some(InheritDecision::Skip) => SubdirPlan::Skip,
        None => {
            if has_conflict(dest_entries) {
                SubdirPlan::NeedsPrompt
            } else {
                SubdirPlan::Link(plan_links(source_entries, dest_entries))
            }
        }
    }
}

/// Names to seed: every source file whose name is absent from the dest.
/// Sorted ascending for deterministic behavior and tests.
pub fn plan_file_seeds(source_files: &[String], dest_files: &[String]) -> Vec<String> {
    let existing: HashSet<&str> = dest_files.iter().map(String::as_str).collect();
    let mut out: Vec<String> = source_files
        .iter()
        .filter(|n| !existing.contains(n.as_str()))
        .cloned()
        .collect();
    out.sort();
    out
}

/// Copy each seedable root-level file (`SEEDED_FILES`) from `source` into
/// `config_dir` when the account doesn't have it yet. One-shot: any existing
/// dest entry (file, dir, or even a dangling symlink) wins and is left
/// untouched, so accounts diverge freely after the first seed. Never writes
/// inside `source`.
pub fn ensure_seeded(source: &Path, config_dir: &Path) -> std::io::Result<()> {
    let source_files: Vec<String> = SEEDED_FILES
        .iter()
        .filter(|f| source.join(f).is_file())
        .map(|f| (*f).to_string())
        .collect();
    let dest_files: Vec<String> = SEEDED_FILES
        .iter()
        .filter(|f| std::fs::symlink_metadata(config_dir.join(f)).is_ok())
        .map(|f| (*f).to_string())
        .collect();
    let plan = plan_file_seeds(&source_files, &dest_files);
    if plan.is_empty() {
        return Ok(());
    }
    std::fs::create_dir_all(config_dir)?;
    for name in plan {
        std::fs::copy(source.join(&name), config_dir.join(&name))?;
    }
    Ok(())
}

/// Result of an inherit pass for one account.
pub struct InheritOutcome {
    /// Subdir names that need a user decision (conflict or stale skip).
    pub needs_prompt: Vec<String>,
}

/// Ensure the account's `config_dir` inherits the shared resources from `source`
/// (e.g. `~/.claude`). For each inherited subdir that exists under `source`,
/// resolves the plan from `decisions` and current dest state and creates links.
/// Subdirs needing a decision are returned in `needs_prompt`; the caller prompts
/// and persists, then calls again. Never writes inside `source`.
pub fn ensure_inherited(
    source: &Path,
    config_dir: &Path,
    decisions: &std::collections::HashMap<String, InheritDecision>,
) -> std::io::Result<InheritOutcome> {
    let mut needs_prompt = Vec::new();
    for sub in INHERITED_SUBDIRS {
        let src_sub = source.join(sub);
        let dest_sub = config_dir.join(sub);
        if !src_sub.is_dir() {
            // Nothing to inherit; links into the now-missing subdir are
            // dangling by definition, so prune them (own files untouched).
            let dest_entries = read_dest_entries(&dest_sub)?;
            prune_links(&src_sub, &dest_sub, &[], &dest_entries, true)?;
            continue;
        }
        let source_entries = read_entry_names(&src_sub)?;
        let dest_entries = read_dest_entries(&dest_sub)?;
        match resolve_subdir(decisions.get(*sub), &source_entries, &dest_entries) {
            SubdirPlan::Skip => {
                prune_links(&src_sub, &dest_sub, &source_entries, &dest_entries, true)?;
            }
            SubdirPlan::NeedsPrompt => needs_prompt.push((*sub).to_string()),
            SubdirPlan::Link(actions) => {
                prune_links(&src_sub, &dest_sub, &source_entries, &dest_entries, false)?;
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

/// Remove the stale inherited links planned by `plan_prune`. Removes the link
/// itself, never what it points at (nothing inside `src_sub` is ever written).
fn prune_links(
    src_sub: &Path,
    dest_sub: &Path,
    source_entries: &[String],
    dest_entries: &[DestEntry],
    prune_all: bool,
) -> std::io::Result<()> {
    for name in plan_prune(src_sub, source_entries, dest_entries, prune_all) {
        remove_link(&dest_sub.join(name))?;
    }
    Ok(())
}

/// Removes one planned link. Re-checks that `path` is still a symlink right
/// before unlinking: if a real file took its place between planning and
/// removal, it is account-owned content and is left alone.
fn remove_link(path: &Path) -> std::io::Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(m) if m.file_type().is_symlink() => remove_symlink(path),
        Ok(_) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

#[cfg(unix)]
fn remove_symlink(path: &Path) -> std::io::Result<()> {
    std::fs::remove_file(path)
}

/// Directory symlinks on Windows must be removed with `remove_dir`.
#[cfg(windows)]
fn remove_symlink(path: &Path) -> std::io::Result<()> {
    std::fs::remove_file(path).or_else(|_| std::fs::remove_dir(path))
}

/// Result of one launch-time `seed_and_apply` pass.
#[derive(Debug)]
pub struct ApplyOutcome {
    /// Subdir names still needing a user decision (conflict or stale skip).
    pub needs_prompt: Vec<String>,
    /// Error message from the best-effort `settings.json` seed, if it failed.
    pub seed_error: Option<String>,
}

/// Error from `seed_and_apply`: the link-inheritance pass failed. Carries the
/// best-effort seed error too, so callers can still surface it — the tray
/// flow always printed the seed warning even when inheritance failed.
#[derive(Debug)]
pub struct ApplyError {
    pub seed_error: Option<String>,
    pub inherit_error: std::io::Error,
}

/// One launch-time inherit pass shared by the tray flow and the `cms` CLI:
/// best-effort root-file seeding, then link inheritance. Does not print and
/// does not persist decisions — the caller decides how to surface
/// `seed_error` and `needs_prompt` (GUI prompt vs stderr warning).
pub fn seed_and_apply(
    source: &Path,
    config_dir: &Path,
    decisions: &std::collections::HashMap<String, InheritDecision>,
) -> Result<ApplyOutcome, ApplyError> {
    let seed_error = ensure_seeded(source, config_dir)
        .err()
        .map(|e| e.to_string());
    match ensure_inherited(source, config_dir, decisions) {
        Ok(outcome) => Ok(ApplyOutcome {
            needs_prompt: outcome.needs_prompt,
            seed_error,
        }),
        Err(inherit_error) => Err(ApplyError {
            seed_error,
            inherit_error,
        }),
    }
}

/// Compute the read-only Inheritance-panel rows for one account: for each
/// inheritable subdir, list source/dest entries and classify via `subdir_status`.
/// Read/list only — never writes inside `source` (e.g. `~/.claude`).
pub fn inherit_status(
    source: &Path,
    config_dir: &Path,
    decisions: &std::collections::HashMap<String, InheritDecision>,
) -> std::io::Result<Vec<InheritSubdirStatus>> {
    let mut rows = Vec::with_capacity(INHERITED_SUBDIRS.len());
    for sub in INHERITED_SUBDIRS {
        let src_sub = source.join(sub);
        let source_entries = if src_sub.is_dir() {
            read_entry_names(&src_sub)?
        } else {
            Vec::new()
        };
        let dest_entries = read_dest_entries(&config_dir.join(sub))?;
        let decision = decisions.get(*sub);
        rows.push(InheritSubdirStatus {
            subdir: (*sub).to_string(),
            status: subdir_status(decision, &source_entries, &dest_entries),
            decision: decision.cloned(),
        });
    }
    Ok(rows)
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
                let link_target = if is_symlink {
                    std::fs::read_link(entry.path()).ok().map(|t| {
                        if t.is_absolute() {
                            t
                        } else {
                            dir.join(t)
                        }
                    })
                } else {
                    None
                };
                entries.push(DestEntry {
                    name,
                    is_symlink,
                    link_target,
                });
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

/// Create a symlink at `dst` pointing to `src`, falling back to a recursive
/// copy when symlink creation fails (e.g. no SeCreateSymbolicLinkPrivilege).
/// Note: a copied fallback entry is a real file, indistinguishable from an
/// account-owned entry on later launches, so it may trigger a one-time
/// Merge/Skip conflict prompt for that subdir on the next launch.
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
        DestEntry {
            name: name.to_string(),
            is_symlink,
            link_target: None,
        }
    }

    /// A symlink entry pointing at `target`.
    fn link(name: &str, target: &str) -> DestEntry {
        DestEntry {
            name: name.to_string(),
            is_symlink: true,
            link_target: Some(PathBuf::from(target)),
        }
    }

    fn pruned(src_sub: &str, src: &[&str], dst: &[DestEntry], all: bool) -> Vec<String> {
        let src: Vec<String> = src.iter().map(|s| s.to_string()).collect();
        plan_prune(Path::new(src_sub), &src, dst, all)
    }

    #[test]
    fn test_should_not_plan_links_for_os_junk() {
        let src: Vec<String> = [".DS_Store", "Thumbs.db", "desktop.ini", "a.md"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let names: Vec<_> = plan_links(&src, &[]).into_iter().map(|a| a.name).collect();
        assert_eq!(names, vec!["a.md"]);
    }

    #[test]
    fn test_should_not_report_conflict_when_dest_only_has_os_junk_files() {
        // Finder drops .DS_Store into the account dir; that is not account content.
        assert!(!has_conflict(&[dest(".DS_Store", false)]));
        assert!(has_conflict(&[
            dest(".DS_Store", false),
            dest("own.md", false)
        ]));
    }

    #[test]
    fn test_should_report_none_status_when_source_has_only_os_junk() {
        let src = vec![".DS_Store".to_string()];
        assert_eq!(subdir_status(None, &src, &[]), InheritStatus::None);
    }

    #[test]
    fn test_should_prune_os_junk_link_when_merge() {
        let dst = vec![
            link(".DS_Store", "/h/.claude/agents/.DS_Store"),
            link("a.md", "/h/.claude/agents/a.md"),
        ];
        // .DS_Store still exists in the source, but is never planned.
        assert_eq!(
            pruned("/h/.claude/agents", &[".DS_Store", "a.md"], &dst, false),
            vec![".DS_Store"]
        );
    }

    #[test]
    fn test_should_prune_only_links_into_source_when_skip() {
        let dst = vec![
            link("a.md", "/h/.claude/agents/a.md"),
            link("dangling.md", "/h/.claude/agents/gone.md"),
            link("foreign.md", "/elsewhere/foreign.md"),
            link("sneaky.md", "/h/.claude/agents/../secret.md"),
            dest("own.md", false),
        ];
        assert_eq!(
            pruned("/h/.claude/agents", &["a.md"], &dst, true),
            vec!["a.md", "dangling.md"]
        );
    }

    #[test]
    fn test_should_never_prune_links_into_a_sibling_prefix_dir() {
        // `agents-extra` shares a string prefix with `agents` but is not inside it.
        let dst = vec![link("x.md", "/h/.claude/agents-extra/x.md")];
        assert!(pruned("/h/.claude/agents", &[], &dst, true).is_empty());
    }

    #[test]
    fn test_should_prune_link_whose_source_entry_vanished_when_merge() {
        let dst = vec![
            link("kept.md", "/h/.claude/agents/kept.md"),
            link("old.md", "/h/.claude/agents/old.md"),
            link("foreign.md", "/elsewhere/foreign.md"),
            dest("own.md", false),
        ];
        assert_eq!(
            pruned("/h/.claude/agents", &["kept.md"], &dst, false),
            vec!["old.md"]
        );
    }

    #[test]
    fn test_should_not_prune_links_without_readable_target() {
        let dst = vec![dest("l.md", true)]; // symlink, target unknown
        assert!(pruned("/h/.claude/agents", &[], &dst, true).is_empty());
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
        assert_eq!(
            plan,
            SubdirPlan::Link(vec![LinkAction {
                name: "a.md".into()
            }])
        );
    }

    #[test]
    fn test_should_skip_when_skip_decision_and_own_entries_present() {
        let src = vec!["a.md".to_string()];
        let plan = resolve_subdir(Some(&InheritDecision::Skip), &src, &[dest("own.md", false)]);
        assert_eq!(plan, SubdirPlan::Skip);
    }

    #[test]
    fn test_should_skip_when_skip_decision_even_without_own_entries() {
        // Sticky decision: a persisted Skip is honored even when no own entries
        // remain. The user manages the choice from the Inheritance panel; we no
        // longer re-prompt when the conflict disappears.
        let src = vec!["a.md".to_string()];
        let plan = resolve_subdir(Some(&InheritDecision::Skip), &src, &[]);
        assert_eq!(plan, SubdirPlan::Skip);
    }

    #[test]
    fn test_should_report_none_status_when_no_source_entries() {
        assert_eq!(subdir_status(None, &[], &[]), InheritStatus::None);
        // Source emptiness dominates even with a decision present.
        assert_eq!(
            subdir_status(Some(&InheritDecision::Merge), &[], &[]),
            InheritStatus::None
        );
    }

    #[test]
    fn test_should_report_inherited_status_when_no_decision_and_no_conflict() {
        let src = vec!["a.md".to_string()];
        assert_eq!(subdir_status(None, &src, &[]), InheritStatus::Inherited);
    }

    #[test]
    fn test_should_report_conflict_status_when_no_decision_and_own_entries() {
        let src = vec!["a.md".to_string()];
        assert_eq!(
            subdir_status(None, &src, &[dest("own.md", false)]),
            InheritStatus::Conflict
        );
    }

    #[test]
    fn test_should_report_inherited_status_when_merge_decision() {
        let src = vec!["a.md".to_string()];
        assert_eq!(
            subdir_status(
                Some(&InheritDecision::Merge),
                &src,
                &[dest("own.md", false)]
            ),
            InheritStatus::Inherited
        );
    }

    #[test]
    fn test_should_report_skipped_status_when_skip_decision() {
        let src = vec!["a.md".to_string()];
        assert_eq!(
            subdir_status(Some(&InheritDecision::Skip), &src, &[dest("own.md", false)]),
            InheritStatus::Skipped
        );
    }

    #[test]
    fn test_should_plan_seed_when_source_has_file_and_dest_lacks_it() {
        let src = vec!["settings.json".to_string()];
        assert_eq!(plan_file_seeds(&src, &[]), vec!["settings.json"]);
    }

    #[test]
    fn test_should_not_plan_seed_when_dest_already_has_file() {
        let src = vec!["settings.json".to_string()];
        let dst = vec!["settings.json".to_string()];
        assert!(plan_file_seeds(&src, &dst).is_empty());
    }

    #[test]
    fn test_should_not_plan_seed_when_source_lacks_file() {
        assert!(plan_file_seeds(&[], &[]).is_empty());
        assert!(plan_file_seeds(&[], &["settings.json".to_string()]).is_empty());
    }

    #[test]
    fn test_should_plan_seeds_sorted_when_multiple_files() {
        let src = vec!["b.json".to_string(), "a.json".to_string()];
        assert_eq!(plan_file_seeds(&src, &[]), vec!["a.json", "b.json"]);
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
        assert!(std::fs::symlink_metadata(&linked)
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(
            std::fs::read_link(&linked).unwrap(),
            source.join("agents").join("a.md")
        );
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
        assert!(cfg.join("agents").join("a.md").exists()); // inherited
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

    /// Sorted relative paths of every file/dir under `root` (for no-write checks).
    fn snapshot(root: &std::path::Path) -> Vec<String> {
        fn walk(dir: &std::path::Path, base: &std::path::Path, out: &mut Vec<String>) {
            for entry in std::fs::read_dir(dir).unwrap() {
                let path = entry.unwrap().path();
                out.push(path.strip_prefix(base).unwrap().display().to_string());
                if path.is_dir() {
                    walk(&path, base, out);
                }
            }
        }
        let mut out = Vec::new();
        walk(root, root, &mut out);
        out.sort();
        out
    }

    fn is_link(p: &std::path::Path) -> bool {
        std::fs::symlink_metadata(p)
            .map(|m| m.file_type().is_symlink())
            .unwrap_or(false)
    }

    fn decide(sub: &str, d: InheritDecision) -> HashMap<String, InheritDecision> {
        HashMap::from([(sub.to_string(), d)])
    }

    #[test]
    fn test_should_remove_inherited_links_when_merge_is_switched_to_skip() {
        let (source, cfg) = fixture("skip_after_merge");
        touch(&source.join("skills").join("a.md"));
        touch(&source.join("skills").join("b.md"));
        touch(&cfg.join("skills").join("own.md"));
        ensure_inherited(&source, &cfg, &decide("skills", InheritDecision::Merge)).unwrap();
        assert!(is_link(&cfg.join("skills").join("a.md")));
        // A symlink to somewhere else: not ours, must survive.
        let foreign = cfg.join("foreign-target.md");
        touch(&foreign);
        symlink(&foreign, cfg.join("skills").join("foreign.md")).unwrap();

        ensure_inherited(&source, &cfg, &decide("skills", InheritDecision::Skip)).unwrap();

        assert!(!is_link(&cfg.join("skills").join("a.md")));
        assert!(!is_link(&cfg.join("skills").join("b.md")));
        assert!(cfg.join("skills").join("own.md").is_file()); // account-owned kept
        assert!(is_link(&cfg.join("skills").join("foreign.md"))); // foreign kept
                                                                  // Pruning removes the link only, never its target inside the source.
        assert!(source.join("skills").join("a.md").is_file());
        assert!(source.join("skills").join("b.md").is_file());
    }

    #[test]
    fn test_should_never_inherit_and_should_clean_existing_os_junk_links() {
        let (source, cfg) = fixture("os_junk");
        touch(&source.join("agents").join("a.md"));
        touch(&source.join("agents").join(".DS_Store"));
        // Pre-existing junk link from before the deny-list, plus a Finder-made
        // real junk file in an account that otherwise has no own content.
        std::fs::create_dir_all(cfg.join("agents")).unwrap();
        symlink(
            source.join("agents").join(".DS_Store"),
            cfg.join("agents").join(".DS_Store"),
        )
        .unwrap();

        let out = ensure_inherited(&source, &cfg, &HashMap::new()).unwrap();

        assert!(out.needs_prompt.is_empty());
        assert!(!is_link(&cfg.join("agents").join(".DS_Store"))); // cleaned
        assert!(is_link(&cfg.join("agents").join("a.md")));
        assert!(source.join("agents").join(".DS_Store").is_file()); // source intact
    }

    #[test]
    fn test_should_not_prompt_when_dest_only_has_real_os_junk() {
        let (source, cfg) = fixture("os_junk_real");
        touch(&source.join("agents").join("a.md"));
        touch(&cfg.join("agents").join(".DS_Store"));
        let out = ensure_inherited(&source, &cfg, &HashMap::new()).unwrap();
        assert!(out.needs_prompt.is_empty());
        assert!(is_link(&cfg.join("agents").join("a.md")));
        assert!(cfg.join("agents").join(".DS_Store").is_file()); // never removed
    }

    #[test]
    fn test_should_prune_link_when_source_entry_deleted_under_merge() {
        let (source, cfg) = fixture("merge_prune_vanished");
        touch(&source.join("agents").join("a.md"));
        touch(&source.join("agents").join("b.md"));
        ensure_inherited(&source, &cfg, &HashMap::new()).unwrap();
        std::fs::remove_file(source.join("agents").join("b.md")).unwrap();

        ensure_inherited(&source, &cfg, &decide("agents", InheritDecision::Merge)).unwrap();

        assert!(is_link(&cfg.join("agents").join("a.md")));
        assert!(!is_link(&cfg.join("agents").join("b.md"))); // dangling link gone
    }

    #[test]
    fn test_should_prune_dangling_links_when_source_subdir_deleted() {
        let (source, cfg) = fixture("source_subdir_deleted");
        touch(&source.join("agents").join("a.md"));
        touch(&cfg.join("agents").join("own.md"));
        ensure_inherited(&source, &cfg, &decide("agents", InheritDecision::Merge)).unwrap();
        assert!(is_link(&cfg.join("agents").join("a.md")));
        let foreign = cfg.join("foreign-target.md");
        touch(&foreign);
        symlink(&foreign, cfg.join("agents").join("foreign.md")).unwrap();
        std::fs::remove_dir_all(source.join("agents")).unwrap();

        let out =
            ensure_inherited(&source, &cfg, &decide("agents", InheritDecision::Merge)).unwrap();

        assert!(out.needs_prompt.is_empty());
        assert!(!is_link(&cfg.join("agents").join("a.md"))); // dangling link gone
        assert!(cfg.join("agents").join("own.md").is_file()); // account-owned kept
        assert!(is_link(&cfg.join("agents").join("foreign.md"))); // foreign kept
    }

    #[test]
    fn test_should_leave_regular_file_in_place_when_remove_link_finds_no_symlink() {
        let (_source, cfg) = fixture("remove_link_guard");
        // A real file that appeared where a link was planned (plan/removal race).
        let own = cfg.join("agents").join("own.md");
        touch(&own);
        remove_link(&own).unwrap();
        assert!(
            own.is_file(),
            "remove_link must never delete account-owned content"
        );
    }

    #[test]
    fn test_should_compute_inherit_status_per_subdir() {
        let (source, cfg) = fixture("status_map");
        touch(&source.join("agents").join("a.md")); // inherited (no conflict)
        touch(&source.join("skills").join("s.md"));
        touch(&cfg.join("skills").join("own.md")); // conflict on skills
                                                   // commands & output-styles: no source subdir → none

        let statuses = inherit_status(&source, &cfg, &HashMap::new()).unwrap();
        let status_of = |name: &str| statuses.iter().find(|s| s.subdir == name).unwrap().status;
        assert_eq!(statuses.len(), INHERITED_SUBDIRS.len());
        assert_eq!(status_of("agents"), InheritStatus::Inherited);
        assert_eq!(status_of("skills"), InheritStatus::Conflict);
        assert_eq!(status_of("commands"), InheritStatus::None);
        assert_eq!(status_of("output-styles"), InheritStatus::None);
    }

    #[test]
    fn test_should_report_recorded_decision_in_status() {
        let (source, cfg) = fixture("status_decision");
        touch(&source.join("agents").join("a.md"));
        touch(&cfg.join("agents").join("own.md"));
        let mut decisions = HashMap::new();
        decisions.insert("agents".to_string(), InheritDecision::Skip);

        let statuses = inherit_status(&source, &cfg, &decisions).unwrap();
        let agents = statuses.iter().find(|s| s.subdir == "agents").unwrap();
        assert_eq!(agents.status, InheritStatus::Skipped);
        assert_eq!(agents.decision, Some(InheritDecision::Skip));
    }

    #[test]
    fn test_should_not_write_source_when_computing_status() {
        let (source, cfg) = fixture("status_nowrite");
        touch(&source.join("agents").join("a.md"));
        touch(&cfg.join("agents").join("own.md")); // conflict, would tempt a write
        let before = snapshot(&source);

        inherit_status(&source, &cfg, &HashMap::new()).unwrap();

        assert_eq!(snapshot(&source), before, "source dir must be untouched");
    }

    #[test]
    fn test_should_stay_skipped_when_skip_and_no_own_entries() {
        // Sticky decision: Skip persists without re-prompting even after the
        // account's own entries are removed (no conflict left).
        let (source, cfg) = fixture("sticky_skip");
        touch(&source.join("agents").join("a.md"));
        std::fs::create_dir_all(cfg.join("agents")).unwrap(); // empty: own entries removed
        let mut decisions = HashMap::new();
        decisions.insert("agents".to_string(), InheritDecision::Skip);
        let out = ensure_inherited(&source, &cfg, &decisions).unwrap();
        assert!(out.needs_prompt.is_empty());
        assert!(!cfg.join("agents").join("a.md").exists()); // not inherited
    }

    #[test]
    fn test_should_not_treat_existing_symlink_as_conflict() {
        let (source, cfg) = fixture("symlink_no_conflict");
        touch(&source.join("agents").join("a.md"));
        // Pre-existing link as if from an earlier run.
        std::fs::create_dir_all(cfg.join("agents")).unwrap();
        symlink(
            source.join("agents").join("a.md"),
            cfg.join("agents").join("a.md"),
        )
        .unwrap();
        let out = ensure_inherited(&source, &cfg, &HashMap::new()).unwrap();
        assert!(out.needs_prompt.is_empty()); // symlink is not account-owned
    }

    #[test]
    fn test_should_copy_settings_when_dest_lacks_it() {
        let (source, cfg) = fixture("seed_basic");
        std::fs::write(source.join("settings.json"), b"{\"statusLine\":{}}").unwrap();

        ensure_seeded(&source, &cfg).unwrap();

        let seeded = cfg.join("settings.json");
        let meta = std::fs::symlink_metadata(&seeded).unwrap();
        assert!(
            meta.file_type().is_file(),
            "must be a real file, not a symlink"
        );
        assert_eq!(
            std::fs::read(&seeded).unwrap(),
            std::fs::read(source.join("settings.json")).unwrap()
        );
    }

    #[test]
    fn test_should_not_overwrite_existing_settings_when_seeding() {
        let (source, cfg) = fixture("seed_existing");
        std::fs::write(source.join("settings.json"), b"{\"from\":\"source\"}").unwrap();
        std::fs::write(cfg.join("settings.json"), b"{\"from\":\"account\"}").unwrap();

        ensure_seeded(&source, &cfg).unwrap();

        assert_eq!(
            std::fs::read(cfg.join("settings.json")).unwrap(),
            b"{\"from\":\"account\"}"
        );
    }

    #[test]
    fn test_should_noop_when_source_has_no_settings() {
        let (source, cfg) = fixture("seed_no_source");
        ensure_seeded(&source, &cfg).unwrap();
        assert!(!cfg.join("settings.json").exists());
    }

    #[test]
    fn test_should_create_config_dir_when_seeding_into_absent_dir() {
        let (source, cfg) = fixture("seed_mkdir");
        std::fs::write(source.join("settings.json"), b"{}").unwrap();
        std::fs::remove_dir_all(&cfg).unwrap();

        ensure_seeded(&source, &cfg).unwrap();

        assert!(cfg.join("settings.json").is_file());
    }

    #[test]
    fn test_should_not_write_source_when_seeding() {
        let (source, cfg) = fixture("seed_nowrite");
        std::fs::write(source.join("settings.json"), b"{}").unwrap();
        let before = snapshot(&source);

        ensure_seeded(&source, &cfg).unwrap();

        assert_eq!(snapshot(&source), before, "source dir must be untouched");
    }

    #[test]
    fn test_should_be_idempotent_when_seeding_twice() {
        let (source, cfg) = fixture("seed_idempotent");
        std::fs::write(source.join("settings.json"), b"{\"v\":1}").unwrap();
        ensure_seeded(&source, &cfg).unwrap();

        // Account diverges, then the source changes: neither must leak across.
        std::fs::write(cfg.join("settings.json"), b"{\"v\":\"mine\"}").unwrap();
        std::fs::write(source.join("settings.json"), b"{\"v\":2}").unwrap();
        ensure_seeded(&source, &cfg).unwrap();

        assert_eq!(
            std::fs::read(cfg.join("settings.json")).unwrap(),
            b"{\"v\":\"mine\"}"
        );
    }

    #[test]
    fn test_should_link_new_source_entry_when_added_after_first_run() {
        let (source, cfg) = fixture("self_healing");
        touch(&source.join("agents").join("a.md"));
        ensure_inherited(&source, &cfg, &HashMap::new()).unwrap();
        assert!(cfg.join("agents").join("a.md").exists());

        // A new resource appears in ~/.claude after the first launch.
        touch(&source.join("agents").join("new.md"));
        let out = ensure_inherited(&source, &cfg, &HashMap::new()).unwrap();
        assert!(out.needs_prompt.is_empty());

        // New entry gets linked; the original link is untouched.
        let new_link = cfg.join("agents").join("new.md");
        assert!(std::fs::symlink_metadata(&new_link)
            .unwrap()
            .file_type()
            .is_symlink());
        assert!(cfg.join("agents").join("a.md").exists());
    }

    #[test]
    fn test_should_report_needs_prompt_and_link_clean_subdirs_when_seed_and_apply() {
        let (source, cfg) = fixture("seed_and_apply");
        // "agents" conflicts (dest has a real file with the same name);
        // "commands" is clean and must link.
        touch(&source.join("agents").join("a.md"));
        touch(&cfg.join("agents").join("a.md"));
        touch(&source.join("commands").join("c.md"));
        touch(&source.join("settings.json"));

        let out = seed_and_apply(&source, &cfg, &HashMap::new()).unwrap();

        assert_eq!(out.needs_prompt, vec!["agents".to_string()]);
        assert_eq!(out.seed_error, None);
        assert!(cfg.join("settings.json").is_file());
        assert!(std::fs::symlink_metadata(cfg.join("commands").join("c.md"))
            .unwrap()
            .file_type()
            .is_symlink());
    }

    #[test]
    fn test_should_carry_seed_error_when_inherit_pass_also_fails() {
        let base = std::env::temp_dir().join("cm_inherit_seed_and_apply_err");
        let _ = std::fs::remove_dir_all(&base);
        let source = base.join("dot-claude");
        std::fs::create_dir_all(&source).unwrap();
        touch(&source.join("agents").join("a.md"));
        touch(&source.join("settings.json"));
        // The account "dir" is a plain file: both the seed and the link pass fail.
        let cfg = base.join("dot-claude-acct");
        std::fs::write(&cfg, b"not a dir").unwrap();

        let err = seed_and_apply(&source, &cfg, &HashMap::new()).unwrap_err();

        assert!(err.seed_error.is_some());
        let _ = std::fs::remove_dir_all(&base);
    }
}
