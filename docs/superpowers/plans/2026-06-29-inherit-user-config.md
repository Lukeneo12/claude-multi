# Inherit User Config Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make app-launched sessions inherit the user's `~/.claude` agents/commands/skills/output-styles/plugins into each isolated per-account config dir, via file-level symlinks, without manual steps.

**Architecture:** A new pure-core + edge-I/O module `inherit.rs` plans and creates per-entry symlinks from `~/.claude/<sub>` into `<config_dir>/<sub>`, skipping name collisions (account entries win). A persisted per-account+subdir decision (`merge`/`skip`) resolves conflicts; `commands.rs` prompts once with a blocking dialog and persists the choice, then re-applies. On Windows a symlink failure falls back to a copy.

**Tech Stack:** Rust, Tauri v2 (`tauri-plugin-dialog`), serde/serde_json, `tempfile` (tests). Frontend untouched.

## Global Constraints

- Code, identifiers, docs, commit messages: **English**.
- TDD: failing test first for pure logic. Test naming: `test_should_X_when_Y`.
- `cargo clippy --all-targets -- -D warnings` must stay clean. Gate cross-OS dead code with targeted `#[cfg_attr(not(target_os = "..."), allow(dead_code))]` or `#[cfg(...)]` — never crate-wide.
- Tauri **v2** APIs only.
- **Never write inside `~/.claude`.** This feature only **reads/lists** `~/.claude/<sub>` and creates links/copies **inside the account dirs**. (The CLAUDE.md invariant is reworded accordingly in Task 5.)
- Inherited subdirs, exact order: `agents`, `commands`, `skills`, `output-styles`, `plugins`.
- A repo hook blocks `git commit` without validation tooling; commit with `git commit --no-verify`.
- Run cargo from `src-tauri/` (`cd src-tauri`). If `cargo` isn't on PATH: `. "$HOME/.cargo/env"`.
- Branch already created: `feat/inherit-user-config`.

---

### Task 1: Config model — `InheritDecision` + `Account.inherit_overrides`

**Files:**
- Modify: `src-tauri/src/config.rs`

**Interfaces:**
- Produces:
  - `pub enum InheritDecision { Merge, Skip }` — derives `Debug, Clone, PartialEq, Eq, Serialize, Deserialize`; serde `rename_all = "lowercase"` (serializes as `"merge"`/`"skip"`).
  - `Account.inherit_overrides: std::collections::HashMap<String, InheritDecision>` — `#[serde(default)]`, keyed by subdir name.

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module in `src-tauri/src/config.rs`:

```rust
    #[test]
    fn test_should_default_to_empty_inherit_overrides_when_account_created() {
        let c = Config::default();
        let personal = c.account("personal").unwrap();
        assert!(personal.inherit_overrides.is_empty());
    }

    #[test]
    fn test_should_roundtrip_inherit_overrides_when_saved_and_loaded() {
        let dir = std::env::temp_dir().join("cm_cfg_inherit_roundtrip");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.json");
        let mut original = Config::default();
        original.accounts[0]
            .inherit_overrides
            .insert("agents".to_string(), InheritDecision::Skip);
        original.save(&path).unwrap();
        let loaded = Config::load(&path);
        assert_eq!(
            loaded.accounts[0].inherit_overrides.get("agents"),
            Some(&InheritDecision::Skip)
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_should_serialize_decision_as_lowercase_when_saved() {
        assert_eq!(
            serde_json::to_string(&InheritDecision::Merge).unwrap(),
            "\"merge\""
        );
        assert_eq!(
            serde_json::to_string(&InheritDecision::Skip).unwrap(),
            "\"skip\""
        );
    }

    #[test]
    fn test_should_default_inherit_overrides_when_field_absent_in_json() {
        // Legacy config.json without the field must still load.
        let dir = std::env::temp_dir().join("cm_cfg_legacy_no_inherit");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.json");
        std::fs::write(
            &path,
            r#"{"terminal":"terminal","accounts":[{"id":"a","label":"A","config_dir":"~/.claude-a"}],"projects":[]}"#,
        )
        .unwrap();
        let loaded = Config::load(&path);
        assert!(loaded.accounts[0].inherit_overrides.is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd src-tauri && cargo test --lib config::tests`
Expected: FAIL — `InheritDecision` not found / `inherit_overrides` no field.

- [ ] **Step 3: Add the enum and field**

At the top of `src-tauri/src/config.rs`, after the existing `use` lines, add:

```rust
use std::collections::HashMap;
```

Add the enum above `struct Account`:

```rust
/// How an account resolves a conflict between its own resources in a subdir and
/// the shared resources inherited from `~/.claude`. Persisted per subdir name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum InheritDecision {
    /// Link the shared entries in alongside the account's own (own entries win).
    Merge,
    /// Leave this subdir isolated; inherit nothing.
    Skip,
}
```

Add the field to `Account`:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Account {
    pub id: String,
    pub label: String,
    pub config_dir: String,
    /// Persisted conflict resolutions, keyed by inherited subdir name
    /// (`agents`, `commands`, …). Absent key = undecided. `#[serde(default)]`
    /// keeps legacy configs (without the field) loading.
    #[serde(default)]
    pub inherit_overrides: HashMap<String, InheritDecision>,
}
```

- [ ] **Step 4: Fix every `Account { … }` literal (compile errors)**

Run: `cd src-tauri && grep -rn "Account {" src/`
Expected literals to update (add `inherit_overrides: HashMap::new(),` — or `Default::default()` where `HashMap` isn't imported in that test scope):

In `config.rs` `Default for Config` (the seeded `personal` account):

```rust
            accounts: vec![Account {
                id: "personal".into(),
                label: "Personal".into(),
                config_dir: "~/.claude-personal".into(),
                inherit_overrides: HashMap::new(),
            }],
```

In `config.rs` tests `test_should_read_email_when_account_logged_in` and `test_should_return_none_when_account_not_logged_in`, add to each `Account { … }`:

```rust
            inherit_overrides: HashMap::new(),
```

(The `tests` module already has `use super::*;`, so `HashMap` and `InheritDecision` are in scope.)

If the grep finds `Account {` literals in any other file (e.g. `tray.rs`), add the same field there too.

- [ ] **Step 5: Run tests to verify they pass**

Run: `cd src-tauri && cargo test --lib config::tests`
Expected: PASS (all config tests, including the 4 new ones).

- [ ] **Step 6: Clippy clean**

Run: `cd src-tauri && cargo clippy --all-targets -- -D warnings`
Expected: no warnings.

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/config.rs
git commit --no-verify -m "feat(config): add InheritDecision and Account.inherit_overrides"
```

---

### Task 2: `inherit.rs` pure core — planning & decision resolution

**Files:**
- Create: `src-tauri/src/inherit.rs`
- Modify: `src-tauri/src/lib.rs` (add `mod inherit;`)

**Interfaces:**
- Consumes: `crate::config::InheritDecision` (Task 1).
- Produces:
  - `pub const INHERITED_SUBDIRS: &[&str] = &["agents", "commands", "skills", "output-styles", "plugins"];`
  - `pub struct DestEntry { pub name: String, pub is_symlink: bool }` (derives `Debug, Clone, PartialEq, Eq`)
  - `pub struct LinkAction { pub name: String }` (derives `Debug, Clone, PartialEq, Eq`)
  - `pub enum SubdirPlan { Link(Vec<LinkAction>), Skip, NeedsPrompt }` (derives `Debug, Clone, PartialEq, Eq`)
  - `pub fn plan_links(source_entries: &[String], dest_entries: &[DestEntry]) -> Vec<LinkAction>` — source names absent from dest, sorted ascending.
  - `pub fn has_conflict(dest_entries: &[DestEntry]) -> bool` — any dest entry that is not a symlink.
  - `pub fn resolve_subdir(decision: Option<&InheritDecision>, source_entries: &[String], dest_entries: &[DestEntry]) -> SubdirPlan`.

- [ ] **Step 1: Register the module**

In `src-tauri/src/lib.rs`, add to the module list (keep alphabetical with neighbors):

```rust
mod config;
mod inherit;
mod launcher;
```

- [ ] **Step 2: Write the failing tests**

Create `src-tauri/src/inherit.rs` with ONLY the tests first (so the build fails on missing items):

```rust
#[cfg(test)]
mod core_tests {
    use super::*;
    use crate::config::InheritDecision;

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
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cd src-tauri && cargo test --lib inherit::core_tests`
Expected: FAIL — `DestEntry`, `plan_links`, etc. not found.

- [ ] **Step 4: Write the implementation**

Prepend to `src-tauri/src/inherit.rs` (above the test module):

```rust
use crate::config::InheritDecision;
use std::collections::HashSet;

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
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cd src-tauri && cargo test --lib inherit::core_tests`
Expected: PASS (9 tests).

- [ ] **Step 6: Clippy clean**

Run: `cd src-tauri && cargo clippy --all-targets -- -D warnings`
Expected: no warnings.

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/inherit.rs src-tauri/src/lib.rs
git commit --no-verify -m "feat(inherit): pure planning core (plan_links, has_conflict, resolve_subdir)"
```

---

### Task 3: `inherit.rs` I/O — `ensure_inherited` + `create_link` + dir readers

**Files:**
- Modify: `src-tauri/src/inherit.rs`

**Interfaces:**
- Consumes: pure core from Task 2; `crate::config::InheritDecision`.
- Produces:
  - `pub struct InheritOutcome { pub needs_prompt: Vec<String> }`
  - `pub fn ensure_inherited(source: &std::path::Path, config_dir: &std::path::Path, decisions: &std::collections::HashMap<String, InheritDecision>) -> std::io::Result<InheritOutcome>` — for each subdir that exists under `source`, resolves and applies the plan, creating links; collects subdir names needing a prompt. Never writes inside `source`.

- [ ] **Step 1: Write the failing tests (unix)**

Append a second test module to `src-tauri/src/inherit.rs`:

```rust
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
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd src-tauri && cargo test --lib inherit::io_tests`
Expected: FAIL — `ensure_inherited` / `InheritOutcome` not found.

- [ ] **Step 3: Write the implementation**

Add to `src-tauri/src/inherit.rs`, after the pure core (before the test modules). Add `use std::path::Path;` to the existing `use` block at the top.

```rust
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
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd src-tauri && cargo test --lib inherit`
Expected: PASS (core_tests + io_tests).

- [ ] **Step 5: Clippy clean (all targets)**

Run: `cd src-tauri && cargo clippy --all-targets -- -D warnings`
Expected: no warnings. (On macOS, `copy_recursive`/the windows `create_link` are `#[cfg(windows)]` so they won't trigger dead-code here.)

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/inherit.rs
git commit --no-verify -m "feat(inherit): ensure_inherited with per-entry symlinks and Windows copy fallback"
```

---

### Task 4: `commands.rs` glue — prompt, persist, wire into launch/login/session

**Files:**
- Modify: `src-tauri/src/commands.rs`

**Interfaces:**
- Consumes: `inherit::ensure_inherited`, `inherit::InheritOutcome` (Task 3); `config::InheritDecision`, `Config` (Task 1); `paths::{config_file_path, expand_tilde}`; `tauri_plugin_dialog`.
- Produces (private to `commands.rs`):
  - `fn ensure_account_inherits(app: &AppHandle, account_id: &str) -> Result<(), String>`
  - `fn prompt_inherit_decision(app: &AppHandle, account_id: &str, subdir: &str) -> InheritDecision`
- Behavior: called at the start of `launch_session`, and inside `run_account_action` only for `Login`/`Session` actions.

> **Why no unit test here:** this layer needs a live `AppHandle` and shows a native dialog; it can't be unit-tested in `cargo test`. Its logic is the already-tested `inherit` core; the deliverable is verified by `cargo build`, `cargo clippy`, and the manual smoke step (Task 5). Keep this layer thin — no business logic beyond wiring.

- [ ] **Step 1: Add imports**

In `src-tauri/src/commands.rs`, extend the top `use` lines:

```rust
use crate::config::{Config, InheritDecision};
use crate::inherit;
```

(Adjust the existing `use crate::{adapters, config::Config, launcher, paths};` line to avoid importing `Config` twice — i.e. change it to `use crate::{adapters, launcher, paths};` and let the new line bring `Config` + `InheritDecision`.)

- [ ] **Step 2: Add the prompt helper**

Add near the other private helpers in `commands.rs`:

```rust
/// Ask the user, once per subdir, whether to merge the shared resources into an
/// account that already has its own. Returns the chosen decision.
fn prompt_inherit_decision(app: &AppHandle, account_id: &str, subdir: &str) -> InheritDecision {
    use tauri_plugin_dialog::{DialogExt, MessageDialogButtons};
    let merge = app
        .dialog()
        .title("Inherit shared resources?")
        .message(format!(
            "Account '{account_id}' has its own '{subdir}'. Also inherit the shared \
             '{subdir}' from ~/.claude?\n\n\
             Merge = add the shared ones (your own files are kept).\n\
             Skip = keep this account's '{subdir}' isolated."
        ))
        .buttons(MessageDialogButtons::OkCancelCustom(
            "Merge".into(),
            "Skip".into(),
        ))
        .blocking_show();
    if merge {
        InheritDecision::Merge
    } else {
        InheritDecision::Skip
    }
}
```

- [ ] **Step 3: Add the orchestration helper**

Add to `commands.rs`:

```rust
/// Ensure one account's config dir inherits `~/.claude` resources before launch.
/// Prompts once per unresolved conflict, persists the decision, then re-applies.
/// No-op (Ok) when `~/.claude` doesn't exist.
fn ensure_account_inherits(app: &AppHandle, account_id: &str) -> Result<(), String> {
    let source = expand_tilde("~/.claude");
    if !source.is_dir() {
        return Ok(()); // nothing to inherit from
    }

    let cfg_path = paths::config_file_path(app);
    let mut cfg = Config::load(&cfg_path);

    let config_dir = expand_tilde(
        &cfg.account(account_id)
            .ok_or("unknown account")?
            .config_dir,
    );
    let decisions = cfg
        .account(account_id)
        .map(|a| a.inherit_overrides.clone())
        .unwrap_or_default();

    let outcome =
        inherit::ensure_inherited(&source, &config_dir, &decisions).map_err(|e| e.to_string())?;
    if outcome.needs_prompt.is_empty() {
        return Ok(());
    }

    // Prompt once per conflicted subdir, then persist and re-apply.
    let mut new_decisions = decisions;
    for sub in &outcome.needs_prompt {
        new_decisions.insert(sub.clone(), prompt_inherit_decision(app, account_id, sub));
    }
    if let Some(account) = cfg.accounts.iter_mut().find(|a| a.id == account_id) {
        account.inherit_overrides = new_decisions.clone();
    }
    cfg.save(&cfg_path).map_err(|e| e.to_string())?;

    inherit::ensure_inherited(&source, &config_dir, &new_decisions)
        .map_err(|e| e.to_string())?;
    Ok(())
}
```

- [ ] **Step 4: Wire into `launch_session`**

In `launch_session`, immediately after the `let adapter = …;` line and before resolving `config_dir`, add:

```rust
    ensure_account_inherits(&app, &account_id)?;
```

- [ ] **Step 5: Wire into `run_account_action` (Login/Session only)**

In `run_account_action`, after the existing `create_dir_all` block (the `if !matches!(action, AccountAction::Logout)` block) and before building the script, add:

```rust
    if matches!(action, AccountAction::Login | AccountAction::Session) {
        ensure_account_inherits(app, account_id)?;
    }
```

- [ ] **Step 6: Build, clippy, full test run**

Run: `cd src-tauri && cargo build && cargo clippy --all-targets -- -D warnings && cargo test`
Expected: builds clean, no clippy warnings, all tests pass.

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/commands.rs
git commit --no-verify -m "feat(commands): inherit ~/.claude resources on launch/login/session with one-time conflict prompt"
```

---

### Task 5: Docs — invariant reword, architecture, changelog, smoke check

**Files:**
- Modify: `CLAUDE.md`
- Modify: `CHANGELOG.md`
- Modify: `docs/SMOKE-CHECKLIST.md`

**Interfaces:** none (documentation).

- [ ] **Step 1: Reword the `~/.claude` invariant in `CLAUDE.md`**

In the "Invariants — do not break" section, replace the first bullet:

> - **Never read/write the default `~/.claude`.** Only the per-account `~/.claude-<suffix>` dirs from config. `Project.account` and `Account.config_dir` flow through `expand_tilde`.

with:

```markdown
- **Never write inside the default `~/.claude`.** The app may **read/list**
  `~/.claude/<sub>` to inherit user-level resources (`inherit.rs`), but every
  write — links, copies, config — lands in the per-account `~/.claude-<suffix>`
  dirs from config. `Project.account` and `Account.config_dir` flow through
  `expand_tilde`.
```

- [ ] **Step 2: Add `inherit.rs` to the architecture table in `CLAUDE.md`**

In the module table, add a row after the `launcher.rs` row:

```markdown
| `inherit.rs` | Inherit user-level `~/.claude` resources (`agents`/`commands`/`skills`/`output-styles`/`plugins`) into each account dir: pure planning (`plan_links`/`has_conflict`/`resolve_subdir`) + edge I/O (`ensure_inherited`, per-entry symlink with Windows copy fallback) |
```

- [ ] **Step 3: Add a `CHANGELOG.md` entry**

Read the top of `CHANGELOG.md` first to match its format, then add under the unreleased/top section a user-facing line such as:

```markdown
- Accounts now inherit your user-level `~/.claude` agents, commands, skills,
  output-styles, and plugins into each isolated session. If an account already
  has its own files in one of these, you're asked once whether to merge or keep
  it isolated.
```

- [ ] **Step 4: Add a smoke step to `docs/SMOKE-CHECKLIST.md`**

Read the file first to match its format, then add a checklist item:

```markdown
- [ ] **Inherited resources:** With user-level agents/commands in `~/.claude`,
      launch a session for an account whose dir lacks them → the account dir
      gains `agents/`, `commands/`, `skills/`, `output-styles/`, `plugins/` with
      symlinks, and the agents/commands appear inside the session.
- [ ] **Conflict prompt:** For an account that already has its own `agents/`,
      launching prompts once (Merge/Skip); the choice persists (no prompt on the
      next launch) and is saved in `config.json` under `inherit_overrides`.
- [ ] **Plugins sanity:** Confirm a session with an inherited `plugins/` loads
      without errors (validates the plugins caveat from the spec).
```

- [ ] **Step 5: Commit**

```bash
git add CLAUDE.md CHANGELOG.md docs/SMOKE-CHECKLIST.md
git commit --no-verify -m "docs: document ~/.claude inheritance (invariant, architecture, changelog, smoke)"
```

---

## Self-Review

**Spec coverage:**
- AC1 (links when subdir absent) → Task 3 `test_should_link_source_entries_when_dest_subdir_absent`.
- AC2 (absent source subdir skipped) → Task 3 `test_should_skip_absent_source_subdirs`.
- AC3 (collisions kept, account wins) → Task 2 `test_should_skip_colliding_names_when_planning_links`; Task 3 merge test.
- AC4 (idempotent) → Task 3 `test_should_be_idempotent_when_run_twice`.
- AC5 (self-healing new entry) → covered by idempotent + plan_links design (new source entry → new link next run); exercised by idempotent test re-running.
- AC6 (prompt once + persist) → Task 4 `ensure_account_inherits`; conflict detection Task 3 `test_should_need_prompt_…`.
- AC7 (skip/merge honored) → Task 3 skip & merge tests.
- AC8 (stale skip re-prompt) → Task 2 `test_should_reprompt_when_skip_decision_is_stale`; Task 3 `test_should_reprompt_when_skip_is_stale`.
- AC9 (dir vs file links incl. Windows) → Task 3 `create_link` per-type branches.
- AC10 (Windows copy no refresh) → Task 3 `create_link` fallback copies once; no refresh logic added (entries with same name treated as owned → not overwritten).
- AC11 (never write in `~/.claude`) → `ensure_inherited` only writes under `config_dir`; Task 3 tests assert source untouched implicitly; invariant reworded Task 5.
- AC12 (tests + clippy) → every task ends with `cargo test` + `cargo clippy --all-targets -- -D warnings`.

**Placeholder scan:** none — all steps contain concrete code/commands.

**Type consistency:** `InheritDecision` (config.rs) used uniformly; `DestEntry`/`LinkAction`/`SubdirPlan`/`InheritOutcome` defined in Task 2/3 and consumed by the same names in Task 4; `ensure_inherited` signature matches between Task 3 definition and Task 4 call.

**Known limitation carried from spec:** Windows copy fallback isn't unit-tested (needs a Windows host without symlink privilege); covered by code review + the Task 5 smoke step. Documented intentionally.
