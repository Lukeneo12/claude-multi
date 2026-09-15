# CLI Execution Layer (`cms`) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A `cms` CLI binary that launches per-account Claude Code sessions inline in the current terminal (IDE terminals like Orca), with full parity with the tray launch flow.

**Architecture:** New auto-discovered bin target `src-tauri/src/bin/cms.rs` linking the existing `claude_multi_lib` crate. All new decision logic lives in a new Tauri-free module `src-tauri/src/cli.rs` (arg parsing, account matching, launch-plan assembly), plus a standalone config-path resolver in `paths.rs` and a shared seed+inherit helper extracted into `inherit.rs`. The bin is edge I/O only: load config → resolve account → session gate → inherit → set env → exec `claude`.

**Tech Stack:** Rust (existing `src-tauri` crate), new dependency `dirs = "6"`. No frontend changes.

**Spec:** `docs/specs/2026-09-14/spec-cli-execution-layer.md`

## Global Constraints

- Code, identifiers, docs, commit messages: **English**.
- Test naming: `test_should_X_when_Y`. TDD: failing test first for pure logic.
- `cargo clippy --all-targets -- -D warnings` must stay clean after every task. Gate cross-OS dead code with targeted `#[cfg_attr(not(target_os = "..."), allow(dead_code))]`, never crate-wide.
- Commit with `git commit --no-verify` (a repo hook blocks commits without validation tooling).
- **Never write inside the default `~/.claude`** — reads/lists only.
- Tauri **v2** APIs only (this plan adds no Tauri API usage at all).
- All `cargo` commands run from `src-tauri/`. If `cargo` isn't on `PATH`: `. "$HOME/.cargo/env"`.
- Working branch: `feature/cli-execution-layer` (already exists, spec committed).

---

### Task 1: Standalone config path resolver (`paths.rs`)

**Files:**
- Modify: `src-tauri/Cargo.toml` (add `dirs = "6"` to `[dependencies]`)
- Modify: `src-tauri/src/paths.rs`
- Test: inline `#[cfg(test)] mod tests` in `src-tauri/src/paths.rs`

**Interfaces:**
- Consumes: nothing new.
- Produces: `pub const APP_IDENTIFIER: &str` and `pub fn standalone_config_file_path() -> Option<PathBuf>` — used by Task 6 (`bin/cms.rs`).

- [ ] **Step 1: Add the `dirs` dependency**

In `src-tauri/Cargo.toml`, append to `[dependencies]`:

```toml
dirs = "6"
```

- [ ] **Step 2: Write the failing tests**

Append inside the existing `mod tests` in `src-tauri/src/paths.rs`:

```rust
    #[test]
    #[cfg(target_os = "macos")]
    fn test_should_match_tauri_app_config_dir_when_on_macos() {
        let home = std::env::var("HOME").unwrap();
        assert_eq!(
            standalone_config_file_path().unwrap(),
            PathBuf::from(home)
                .join("Library/Application Support")
                .join("com.lucasdonadio.claude-multi")
                .join("config.json")
        );
    }

    #[test]
    fn test_should_end_with_identifier_and_filename_when_resolving_standalone_path() {
        let p = standalone_config_file_path().unwrap();
        assert!(p.ends_with(format!("{APP_IDENTIFIER}/config.json")));
    }
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cd src-tauri && cargo test paths::`
Expected: compile error — `standalone_config_file_path` and `APP_IDENTIFIER` not found.

- [ ] **Step 4: Implement**

Add to `src-tauri/src/paths.rs` (below `config_file_path`):

```rust
/// Bundle identifier — must match `identifier` in `tauri.conf.json`. Tauri
/// derives `app_config_dir` from it, and the standalone resolver below must
/// agree with that path or the CLI would read a different config.
pub const APP_IDENTIFIER: &str = "com.lucasdonadio.claude-multi";

/// Tauri-free equivalent of `config_file_path`, for the `cms` CLI bin.
/// `dirs::config_dir()` matches Tauri's `app_config_dir` base on all three
/// desktop OSes: macOS `~/Library/Application Support`, Linux XDG config dir,
/// Windows Roaming AppData.
pub fn standalone_config_file_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join(APP_IDENTIFIER).join("config.json"))
}
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cd src-tauri && cargo test paths::`
Expected: PASS (4 tests in `paths::tests`).

- [ ] **Step 6: Clippy + commit**

Run: `cd src-tauri && cargo clippy --all-targets -- -D warnings`
Expected: clean.

```bash
git add src-tauri/Cargo.toml src-tauri/Cargo.lock src-tauri/src/paths.rs
git commit --no-verify -m "feat: add standalone config path resolver for CLI"
```

---

### Task 2: CLI arg parsing (`cli.rs`, new module)

**Files:**
- Create: `src-tauri/src/cli.rs`
- Modify: `src-tauri/src/lib.rs` (register the module)
- Test: inline `#[cfg(test)] mod tests` in `src-tauri/src/cli.rs`

**Interfaces:**
- Consumes: nothing.
- Produces: `pub enum CliCommand { Help { explicit: bool }, List, Launch { query: String, claude_args: Vec<String> } }` and `pub fn parse_cli_args(args: &[String]) -> CliCommand` — used by Task 6.

- [ ] **Step 1: Create the module skeleton and register it**

Create `src-tauri/src/cli.rs`:

```rust
//! Pure decision logic for the `cms` CLI bin: arg parsing, account matching,
//! and launch-plan assembly. No I/O — the bin (`src/bin/cms.rs`) owns all
//! printing, process spawning, and filesystem access.
```

In `src-tauri/src/lib.rs`, the module list currently reads `mod adapters;` … `mod usage;`. Add (alphabetical, before `mod commands;`):

```rust
mod cli;
```

(It becomes `pub mod` in Task 6 when the bin needs it; keeping it private until then avoids `-D warnings` dead-code noise being masked. If clippy flags the unused module before Task 6, silence per-item with `#[allow(dead_code)]` on the items and remove those in Task 6 — do NOT allow crate-wide.)

- [ ] **Step 2: Write the failing tests**

Append to `src-tauri/src/cli.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn test_should_return_implicit_help_when_no_args() {
        assert_eq!(parse_cli_args(&[]), CliCommand::Help { explicit: false });
    }

    #[test]
    fn test_should_return_explicit_help_when_help_flag() {
        assert_eq!(
            parse_cli_args(&args(&["--help"])),
            CliCommand::Help { explicit: true }
        );
        assert_eq!(
            parse_cli_args(&args(&["-h"])),
            CliCommand::Help { explicit: true }
        );
    }

    #[test]
    fn test_should_return_list_when_list_flag() {
        assert_eq!(parse_cli_args(&args(&["--list"])), CliCommand::List);
        assert_eq!(parse_cli_args(&args(&["-l"])), CliCommand::List);
    }

    #[test]
    fn test_should_pass_trailing_args_through_when_launching() {
        assert_eq!(
            parse_cli_args(&args(&["dino", "--resume", "-p", "x y"])),
            CliCommand::Launch {
                query: "dino".to_string(),
                claude_args: args(&["--resume", "-p", "x y"]),
            }
        );
    }

    #[test]
    fn test_should_launch_with_empty_claude_args_when_only_account_given() {
        assert_eq!(
            parse_cli_args(&args(&["personal"])),
            CliCommand::Launch {
                query: "personal".to_string(),
                claude_args: vec![],
            }
        );
    }
}
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cd src-tauri && cargo test cli::`
Expected: compile error — `CliCommand`/`parse_cli_args` not found.

- [ ] **Step 4: Implement**

Add to `src-tauri/src/cli.rs` (above the tests):

```rust
/// What the `cms` invocation asks for. `Help { explicit }` distinguishes
/// `cms --help` (exit 0) from `cms` with no args (usage error, exit != 0).
#[derive(Debug, PartialEq)]
pub enum CliCommand {
    Help { explicit: bool },
    List,
    Launch {
        query: String,
        claude_args: Vec<String>,
    },
}

/// Parses `cms` args (without the program name). The first arg selects the
/// command; everything after an account query passes through to `claude`
/// verbatim, in order.
pub fn parse_cli_args(args: &[String]) -> CliCommand {
    match args.first().map(String::as_str) {
        None => CliCommand::Help { explicit: false },
        Some("--help") | Some("-h") => CliCommand::Help { explicit: true },
        Some("--list") | Some("-l") => CliCommand::List,
        Some(query) => CliCommand::Launch {
            query: query.to_string(),
            claude_args: args[1..].to_vec(),
        },
    }
}
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cd src-tauri && cargo test cli::`
Expected: PASS (5 tests).

- [ ] **Step 6: Clippy + commit**

Run: `cd src-tauri && cargo clippy --all-targets -- -D warnings`
Expected: clean (add per-item `#[allow(dead_code)]` if flagged; Task 6 removes them).

```bash
git add src-tauri/src/cli.rs src-tauri/src/lib.rs
git commit --no-verify -m "feat: add cms CLI arg parsing"
```

---

### Task 3: Fuzzy account matching (`cli.rs`)

**Files:**
- Modify: `src-tauri/src/cli.rs`
- Test: same file, same `mod tests`

**Interfaces:**
- Consumes: `crate::config::Account` (existing).
- Produces: `pub enum AccountMatchError { NoMatch, Ambiguous(Vec<String>) }` (ids of candidates) and `pub fn match_account<'a>(accounts: &'a [Account], query: &str) -> Result<&'a Account, AccountMatchError>` — used by Task 6.

- [ ] **Step 1: Write the failing tests**

Append inside `mod tests` in `src-tauri/src/cli.rs`:

```rust
    use crate::config::{Account, UsageLimits};
    use std::collections::HashMap;

    fn account(id: &str, label: &str) -> Account {
        Account {
            id: id.to_string(),
            label: label.to_string(),
            config_dir: format!("~/.claude-{id}"),
            inherit_overrides: HashMap::new(),
            usage_limits: UsageLimits::default(),
        }
    }

    fn fixture_accounts() -> Vec<Account> {
        vec![account("personal", "Personal"), account("a1", "Dinocloud")]
    }

    #[test]
    fn test_should_match_by_exact_id_when_query_is_id() {
        let accounts = fixture_accounts();
        assert_eq!(match_account(&accounts, "a1").unwrap().id, "a1");
    }

    #[test]
    fn test_should_match_case_insensitively_when_query_is_label() {
        let accounts = fixture_accounts();
        assert_eq!(match_account(&accounts, "DINOCLOUD").unwrap().id, "a1");
    }

    #[test]
    fn test_should_match_by_unique_prefix_when_query_shortened() {
        let accounts = fixture_accounts();
        assert_eq!(match_account(&accounts, "dino").unwrap().id, "a1");
        assert_eq!(match_account(&accounts, "per").unwrap().id, "personal");
    }

    #[test]
    fn test_should_prefer_exact_match_when_query_is_also_prefix_of_other() {
        // "work" is exact for one account and a prefix of "workshop".
        let accounts = vec![account("work", "Work"), account("workshop", "Workshop")];
        assert_eq!(match_account(&accounts, "work").unwrap().id, "work");
    }

    #[test]
    fn test_should_error_no_match_when_query_matches_nothing() {
        let accounts = fixture_accounts();
        assert_eq!(
            match_account(&accounts, "x").unwrap_err(),
            AccountMatchError::NoMatch
        );
    }

    #[test]
    fn test_should_error_no_match_when_query_empty() {
        let accounts = fixture_accounts();
        assert_eq!(
            match_account(&accounts, "").unwrap_err(),
            AccountMatchError::NoMatch
        );
    }

    #[test]
    fn test_should_error_ambiguous_with_candidate_ids_when_prefix_matches_many() {
        let accounts = vec![account("dev1", "Dev One"), account("dev2", "Dev Two")];
        assert_eq!(
            match_account(&accounts, "dev").unwrap_err(),
            AccountMatchError::Ambiguous(vec!["dev1".to_string(), "dev2".to_string()])
        );
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd src-tauri && cargo test cli::`
Expected: compile error — `match_account`/`AccountMatchError` not found.

- [ ] **Step 3: Implement**

Add to `src-tauri/src/cli.rs`:

```rust
use crate::config::Account;

/// Why an account query failed to resolve. `Ambiguous` carries the candidate
/// ids so the caller can list them.
#[derive(Debug, PartialEq)]
pub enum AccountMatchError {
    NoMatch,
    Ambiguous(Vec<String>),
}

/// Resolves a user-typed query against configured accounts, case-insensitively
/// over both `id` and `label`. An exact match wins outright; otherwise a
/// unique prefix match resolves, and multiple prefix candidates are an error.
pub fn match_account<'a>(
    accounts: &'a [Account],
    query: &str,
) -> Result<&'a Account, AccountMatchError> {
    let q = query.to_lowercase();
    if q.is_empty() {
        return Err(AccountMatchError::NoMatch);
    }
    if let Some(exact) = accounts
        .iter()
        .find(|a| a.id.to_lowercase() == q || a.label.to_lowercase() == q)
    {
        return Ok(exact);
    }
    let candidates: Vec<&Account> = accounts
        .iter()
        .filter(|a| {
            a.id.to_lowercase().starts_with(&q) || a.label.to_lowercase().starts_with(&q)
        })
        .collect();
    match candidates.len() {
        0 => Err(AccountMatchError::NoMatch),
        1 => Ok(candidates[0]),
        _ => Err(AccountMatchError::Ambiguous(
            candidates.iter().map(|a| a.id.clone()).collect(),
        )),
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd src-tauri && cargo test cli::`
Expected: PASS (12 tests).

- [ ] **Step 5: Clippy + commit**

Run: `cd src-tauri && cargo clippy --all-targets -- -D warnings`

```bash
git add src-tauri/src/cli.rs
git commit --no-verify -m "feat: add fuzzy account matching for cms CLI"
```

---

### Task 4: Launch plan assembly (`cli.rs` + expose env-var table)

**Files:**
- Modify: `src-tauri/src/launcher.rs:29` (make `PER_ACCOUNT_ENV_VARS` `pub`)
- Modify: `src-tauri/src/cli.rs`
- Test: same `mod tests` in `cli.rs`

**Interfaces:**
- Consumes: `crate::paths::expand_tilde` (existing), `crate::launcher::PER_ACCOUNT_ENV_VARS` (existing const, currently private: `const PER_ACCOUNT_ENV_VARS: &[(&str, &str)] = &[("GH_CONFIG_DIR", "gh")];`).
- Produces: `pub struct LaunchPlan { pub env: Vec<(String, String)> }` and `pub fn launch_plan(account: &Account) -> LaunchPlan` — used by Task 6. Env values are native paths built with `PathBuf::join` (no shell text, no escaping — nothing is interpolated into a script).

- [ ] **Step 1: Write the failing tests**

Append inside `mod tests` in `src-tauri/src/cli.rs`:

```rust
    #[test]
    fn test_should_expand_tilde_when_building_launch_plan() {
        let plan = launch_plan(&account("personal", "Personal"));
        let home = std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .unwrap();
        let expected_dir = std::path::PathBuf::from(&home).join(".claude-personal");
        assert_eq!(
            plan.env[0],
            (
                "CLAUDE_CONFIG_DIR".to_string(),
                expected_dir.to_string_lossy().into_owned()
            )
        );
    }

    #[test]
    fn test_should_include_per_account_env_vars_when_building_launch_plan() {
        let plan = launch_plan(&account("personal", "Personal"));
        let gh = plan
            .env
            .iter()
            .find(|(k, _)| k == "GH_CONFIG_DIR")
            .expect("GH_CONFIG_DIR present");
        let claude_dir = &plan.env[0].1;
        assert_eq!(
            gh.1,
            std::path::Path::new(claude_dir)
                .join("gh")
                .to_string_lossy()
                .into_owned()
        );
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd src-tauri && cargo test cli::`
Expected: compile error — `launch_plan`/`LaunchPlan` not found.

- [ ] **Step 3: Implement**

In `src-tauri/src/launcher.rs`, change the const declaration (keep the doc comment above it as-is):

```rust
pub const PER_ACCOUNT_ENV_VARS: &[(&str, &str)] = &[("GH_CONFIG_DIR", "gh")];
```

Add to `src-tauri/src/cli.rs`:

```rust
/// Environment the `cms` bin sets on the `claude` child process. Values are
/// native paths (via `PathBuf::join`) — no shell text is generated, so the
/// script-escaping invariant does not apply here.
pub struct LaunchPlan {
    pub env: Vec<(String, String)>,
}

/// Builds the env for launching `claude` under `account`:
/// `CLAUDE_CONFIG_DIR` first, then one entry per
/// `launcher::PER_ACCOUNT_ENV_VARS` (`<config_dir>/<subdir>`), mirroring the
/// tray's script builders.
pub fn launch_plan(account: &Account) -> LaunchPlan {
    let dir = crate::paths::expand_tilde(&account.config_dir);
    let mut env = vec![(
        "CLAUDE_CONFIG_DIR".to_string(),
        dir.to_string_lossy().into_owned(),
    )];
    for (var, subdir) in crate::launcher::PER_ACCOUNT_ENV_VARS {
        env.push((
            (*var).to_string(),
            dir.join(subdir).to_string_lossy().into_owned(),
        ));
    }
    LaunchPlan { env }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd src-tauri && cargo test`
Expected: full suite PASS (launcher tests must be untouched by the visibility change).

- [ ] **Step 5: Clippy + commit**

Run: `cd src-tauri && cargo clippy --all-targets -- -D warnings`

```bash
git add src-tauri/src/cli.rs src-tauri/src/launcher.rs
git commit --no-verify -m "feat: add launch plan assembly for cms CLI"
```

---

### Task 5: Shared seed+inherit helper (`inherit.rs`) and tray refactor

**Files:**
- Modify: `src-tauri/src/inherit.rs`
- Modify: `src-tauri/src/commands.rs:36-75` (`ensure_account_inherits`)
- Test: `io_tests` module in `src-tauri/src/inherit.rs`

**Interfaces:**
- Consumes: existing `inherit::ensure_seeded(&Path, &Path) -> std::io::Result<()>`, `inherit::ensure_inherited(&Path, &Path, &HashMap<String, InheritDecision>) -> std::io::Result<InheritOutcome>`.
- Produces: `pub struct ApplyOutcome { pub needs_prompt: Vec<String>, pub seed_error: Option<String> }` and `pub fn seed_and_apply(source: &Path, config_dir: &Path, decisions: &HashMap<String, InheritDecision>) -> std::io::Result<ApplyOutcome>` — used by `commands.rs` (this task) and Task 6. No printing inside — callers surface `seed_error` / `needs_prompt` their own way (GUI prompt vs stderr warning).

- [ ] **Step 1: Write the failing test**

Append inside `mod io_tests` in `src-tauri/src/inherit.rs` (reuse the existing `fixture`/`touch` helpers in that module):

```rust
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
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd src-tauri && cargo test inherit::`
Expected: compile error — `seed_and_apply`/`ApplyOutcome` not found.

- [ ] **Step 3: Implement the helper**

Add to `src-tauri/src/inherit.rs`, right below `ensure_inherited`:

```rust
/// Result of one launch-time `seed_and_apply` pass.
pub struct ApplyOutcome {
    /// Subdir names still needing a user decision (conflict or stale skip).
    pub needs_prompt: Vec<String>,
    /// Error message from the best-effort `settings.json` seed, if it failed.
    pub seed_error: Option<String>,
}

/// One launch-time inherit pass shared by the tray flow and the `cms` CLI:
/// best-effort root-file seeding, then link inheritance. Does not print and
/// does not persist decisions — the caller decides how to surface
/// `seed_error` and `needs_prompt` (GUI prompt vs stderr warning).
pub fn seed_and_apply(
    source: &Path,
    config_dir: &Path,
    decisions: &std::collections::HashMap<String, InheritDecision>,
) -> std::io::Result<ApplyOutcome> {
    let seed_error = ensure_seeded(source, config_dir).err().map(|e| e.to_string());
    let outcome = ensure_inherited(source, config_dir, decisions)?;
    Ok(ApplyOutcome {
        needs_prompt: outcome.needs_prompt,
        seed_error,
    })
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd src-tauri && cargo test inherit::`
Expected: PASS.

- [ ] **Step 5: Refactor `ensure_account_inherits` to use the helper**

In `src-tauri/src/commands.rs`, inside `ensure_account_inherits`, replace this block:

```rust
    // Best-effort: seeding settings.json is a convenience and must never block
    // the session launch.
    if let Err(e) = inherit::ensure_seeded(&source, &config_dir) {
        eprintln!("settings.json seed failed for account '{account_id}': {e}");
    }

    let outcome =
        inherit::ensure_inherited(&source, &config_dir, &decisions).map_err(|e| e.to_string())?;
    if outcome.needs_prompt.is_empty() {
        return Ok(());
    }
```

with:

```rust
    // Best-effort: seeding settings.json is a convenience and must never block
    // the session launch.
    let outcome = inherit::seed_and_apply(&source, &config_dir, &decisions)
        .map_err(|e| e.to_string())?;
    if let Some(e) = outcome.seed_error {
        eprintln!("settings.json seed failed for account '{account_id}': {e}");
    }
    if outcome.needs_prompt.is_empty() {
        return Ok(());
    }
```

The rest of the function (prompt loop over `outcome.needs_prompt`, persist, re-apply via `inherit::ensure_inherited`) stays unchanged.

- [ ] **Step 6: Run the full suite + clippy**

Run: `cd src-tauri && cargo test && cargo clippy --all-targets -- -D warnings`
Expected: all PASS, clippy clean. This is a behavior-preserving refactor — no tray test may change.

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/inherit.rs src-tauri/src/commands.rs
git commit --no-verify -m "refactor: extract shared seed_and_apply inherit pass"
```

---

### Task 6: The `cms` bin (`src/bin/cms.rs`) + public modules

**Files:**
- Create: `src-tauri/src/bin/cms.rs`
- Modify: `src-tauri/src/lib.rs` (make modules `pub`)
- Test: manual verification (edge I/O; all decision logic already unit-tested in Tasks 1–5)

**Interfaces:**
- Consumes (all from `claude_multi_lib`): `cli::{parse_cli_args, CliCommand, match_account, AccountMatchError, launch_plan}`, `config::Config` (+ `Account::logged_in_email`), `paths::standalone_config_file_path`, `paths::expand_tilde`, `session::ensure_session_usable`, `inherit::seed_and_apply`.
- Produces: the `cms` binary (cargo auto-discovers `src/bin/cms.rs`; no `[[bin]]` section needed).

- [ ] **Step 1: Make the lib modules public**

In `src-tauri/src/lib.rs`, change the module declarations to:

```rust
pub mod adapters;
pub mod cli;
pub mod commands;
pub mod config;
pub mod inherit;
pub mod launcher;
pub mod paths;
pub mod session;
pub mod tray;
pub mod usage;
```

(Making them all `pub` is deliberate: it keeps the list uniform and clippy will not flag pub items as dead code, which also lets you remove any `#[allow(dead_code)]` added in Tasks 2–4.)

- [ ] **Step 2: Write the bin**

Create `src-tauri/src/bin/cms.rs`:

```rust
//! `cms` — launch a per-account Claude Code session inline in the current
//! terminal (for IDE terminals where the tray's new-window launch doesn't
//! fit). Edge I/O only; decision logic lives in `claude_multi_lib::cli`.

use claude_multi_lib::cli::{
    launch_plan, match_account, parse_cli_args, AccountMatchError, CliCommand,
};
use claude_multi_lib::config::Config;
use claude_multi_lib::{inherit, paths, session};
use std::process::ExitCode;

fn usage() -> String {
    "Usage: cms <account> [claude args...]   launch claude here under <account>\n       cms --list | -l                  list configured accounts\n       cms --help | -h                  show this help\n\n<account> matches account id or label, case-insensitively, by unique prefix."
        .to_string()
}

fn account_list(cfg: &Config) -> String {
    cfg.accounts
        .iter()
        .map(|a| {
            let auth = a
                .logged_in_email()
                .unwrap_or_else(|| "not logged in".to_string());
            format!("  {:<12} {:<16} {auth}", a.id, a.label)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn load_config() -> Result<Config, String> {
    let path = paths::standalone_config_file_path()
        .ok_or("could not resolve the user config directory")?;
    if !path.is_file() {
        return Err(format!(
            "claude-multi is not configured yet (missing {}).\nOpen the claude-multi app once to create your accounts.",
            path.display()
        ));
    }
    Ok(Config::load(&path))
}

fn run() -> Result<ExitCode, String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match parse_cli_args(&args) {
        CliCommand::Help { explicit } => {
            let text = match load_config() {
                Ok(cfg) => format!("{}\n\nAccounts:\n{}", usage(), account_list(&cfg)),
                Err(_) => usage(),
            };
            if explicit {
                println!("{text}");
                Ok(ExitCode::SUCCESS)
            } else {
                eprintln!("{text}");
                Ok(ExitCode::from(2))
            }
        }
        CliCommand::List => {
            let cfg = load_config()?;
            println!("{}", account_list(&cfg));
            Ok(ExitCode::SUCCESS)
        }
        CliCommand::Launch { query, claude_args } => {
            let cfg = load_config()?;
            let account = match_account(&cfg.accounts, &query).map_err(|e| {
                let reason = match e {
                    AccountMatchError::NoMatch => format!("no account matches '{query}'"),
                    AccountMatchError::Ambiguous(ids) => {
                        format!("'{query}' is ambiguous ({})", ids.join(", "))
                    }
                };
                format!("{reason}. Available accounts:\n{}", account_list(&cfg))
            })?;

            session::ensure_session_usable(account)?;

            // Same inherit pass as the tray launch; the CLI never opens a GUI
            // prompt and never persists decisions — undecided conflicts are
            // skipped for this launch and resolved from the tray.
            let source = paths::expand_tilde("~/.claude");
            if source.is_dir() {
                let config_dir = paths::expand_tilde(&account.config_dir);
                let outcome =
                    inherit::seed_and_apply(&source, &config_dir, &account.inherit_overrides)
                        .map_err(|e| e.to_string())?;
                if let Some(e) = outcome.seed_error {
                    eprintln!("warning: settings.json seed failed: {e}");
                }
                for sub in &outcome.needs_prompt {
                    eprintln!(
                        "warning: '{sub}' has a conflict with ~/.claude and no saved decision; launching without inheriting it. Launch once from the tray to resolve."
                    );
                }
            }

            exec_claude(account, &claude_args)
        }
    }
}

#[cfg(unix)]
fn exec_claude(
    account: &claude_multi_lib::config::Account,
    claude_args: &[String],
) -> Result<ExitCode, String> {
    use std::os::unix::process::CommandExt;
    let mut cmd = std::process::Command::new("claude");
    cmd.args(claude_args);
    for (k, v) in launch_plan(account).env {
        cmd.env(k, v);
    }
    // exec only returns on failure.
    Err(format!("failed to launch claude: {}", cmd.exec()))
}

#[cfg(not(unix))]
fn exec_claude(
    account: &claude_multi_lib::config::Account,
    claude_args: &[String],
) -> Result<ExitCode, String> {
    let mut cmd = std::process::Command::new("claude");
    cmd.args(claude_args);
    for (k, v) in launch_plan(account).env {
        cmd.env(k, v);
    }
    let status = cmd
        .status()
        .map_err(|e| format!("failed to launch claude: {e}"))?;
    Ok(ExitCode::from(status.code().unwrap_or(1) as u8))
}

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(msg) => {
            eprintln!("cms: {msg}");
            ExitCode::FAILURE
        }
    }
}
```

- [ ] **Step 3: Build, test, clippy**

Run: `cd src-tauri && cargo build --bin cms && cargo test && cargo clippy --all-targets -- -D warnings`
Expected: all clean. Remove any `#[allow(dead_code)]` left from Tasks 2–4 (pub items are no longer flagged).

- [ ] **Step 4: Manual verification (AC1, AC2, AC3, AC5, AC7)**

```bash
cd src-tauri
cargo run --bin cms                       # usage + account list on stderr, exit 2
cargo run --bin cms -- --list             # accounts with emails, exit 0
cargo run --bin cms -- nope; echo "exit=$?"   # error listing accounts, exit 1
cargo run --bin cms -- dino --help 2>&1 | head -1   # only if a Dinocloud session is usable
```

For the launch case, verify from inside the started claude session (or with `cms <account> --help`, which execs `claude --help` under the env) that no new terminal window opened and the process replaced `cms`. If an account is logged out, verify the gate message appears and `claude` did not start.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/bin/cms.rs src-tauri/src/lib.rs src-tauri/src/cli.rs
git commit --no-verify -m "feat: add cms bin launching sessions inline in the current terminal"
```

---

### Task 7: Install script + docs

**Files:**
- Modify: `package.json` (add `install-cli` script)
- Modify: `CHANGELOG.md` (user-facing entry)
- Modify: `docs/SMOKE-CHECKLIST.md` (manual verification section)
- Modify: `README.md` (usage section, if the README documents features)

**Interfaces:**
- Consumes: the `cms` bin from Task 6.
- Produces: `npm run install-cli` → `~/.local/bin/cms` symlink.

- [ ] **Step 1: Add the npm script**

In `package.json` `"scripts"`, after `"tauri": "tauri"`:

```json
    "install-cli": "cd src-tauri && cargo build --release --bin cms && mkdir -p \"$HOME/.local/bin\" && ln -sf \"$(pwd)/target/release/cms\" \"$HOME/.local/bin/cms\" && echo 'cms installed to ~/.local/bin/cms'"
```

(POSIX-only by design; Windows users run `cargo build --release --bin cms` and add `src-tauri\target\release` to `PATH` — say exactly that in the README.)

- [ ] **Step 2: Run it and verify (AC9)**

Run: `npm run install-cli && ~/.local/bin/cms --list`
Expected: build succeeds, symlink exists, account list prints.

- [ ] **Step 3: CHANGELOG entry**

Add under the unreleased/top section of `CHANGELOG.md` (follow the file's existing format):

```markdown
- **`cms` CLI**: launch a session inline in the current terminal (IDE integrated
  terminals) with `cms <account>` — fuzzy account matching, cwd as project, same
  session gate and `~/.claude` inheritance as the tray. Install with
  `npm run install-cli`.
```

- [ ] **Step 4: Smoke checklist section**

Append to `docs/SMOKE-CHECKLIST.md` (follow the file's existing format):

```markdown
## cms CLI

- [ ] `cms --list` shows all configured accounts with logged-in emails.
- [ ] `cms <prefix>` from a project dir starts claude inline (no new window) under the right account (`/status` inside claude shows the account's email).
- [ ] `cms nope` exits non-zero listing available accounts.
- [ ] `cms <logged-out-account>` refuses with the tray login hint before starting claude.
- [ ] Trailing args pass through: `cms <account> --help` prints claude's help.
```

- [ ] **Step 5: README mention**

In `README.md`, add a short "CLI (`cms`)" section near the usage docs: one paragraph (what it is, `cms <account>` example inside an IDE terminal), the `npm run install-cli` install line, and the Windows note from Step 1.

- [ ] **Step 6: Final verification + commit**

Run: `cd src-tauri && cargo test && cargo clippy --all-targets -- -D warnings && cd .. && npm run build`
Expected: everything clean (npm build confirms no frontend impact).

```bash
git add package.json CHANGELOG.md docs/SMOKE-CHECKLIST.md README.md
git commit --no-verify -m "feat: add install-cli script and cms docs"
```
