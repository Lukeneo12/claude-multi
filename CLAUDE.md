# claude-multi — Project Guide

Cross-OS system-tray app to launch interactive Claude Code sessions under multiple accounts, each isolated in its own `CLAUDE_CONFIG_DIR`. Tauri v2 + Rust backend + React/TypeScript Preferences window.

## Build / test / lint

```sh
# Run the app (compiles Rust, opens the tray app)
npm run tauri dev

# Rust backend (run from src-tauri/)
cd src-tauri
cargo test                              # unit tests (TDD; pure logic is well covered)
cargo clippy --all-targets -- -D warnings   # must be clean — CI gate

# Frontend
npm run build                           # tsc + vite build; must have 0 TS errors
```

If `cargo` isn't on `PATH`: `. "$HOME/.cargo/env"`.

## Architecture

Single-responsibility modules in `src-tauri/src/`:

| File | Responsibility |
|---|---|
| `paths.rs` | `expand_tilde`, `config_file_path` (Tauri app-config dir), `APP_IDENTIFIER` (must match `tauri.conf.json`) + `standalone_config_file_path` (same file resolved without a Tauri handle, for the CLI) |
| `config.rs` | `Account`/`Project`/`Config` models, defaults, `Account::logged_in_email` (reads `<config_dir>/.claude.json`), pure `validate_unique_config_dirs` (no two accounts may share a `config_dir`; compared after `expand_tilde`, ignoring trailing separators and case) called by `save_config` before persisting. Loading: `LoadOutcome` (`Loaded`/`Missing`/`Corrupt`/`Unreadable`) from side-effect-free `Config::read`; `load` (lenient → default, tray hover, never writes), `load_strict` (mutating/launch paths: a corrupt file is copied to a backup and an error returned, never overwritten; an unreadable file (permissions, transient I/O) is only reported, no backup), `try_load` (`cms`: error instead of inventing the default account). `save` is atomic (temp file + rename, `0600` on Unix). `ConfigStore` (`Mutex` + path, managed Tauri state): `load`/`update` serialize every read-modify-write. `merge_inherit_overrides`/`apply_inherit_decisions`: pure helpers so Preferences Save keeps inherit decisions made out-of-band |
| `launcher.rs` | Pure builders for ephemeral scripts (`build_script`/`build_login_script`/`build_logout_script`/`build_relogin_script`) + `write_script` (atomic, restrictive perms via `tempfile`) + quote-escaping helpers + `ScriptKind`; manual clipboard-fallback builders (`manual_command`/`manual_login_command`/`manual_logout_command`/`manual_relogin_command`) take a `ScriptKind` and escape per shell (POSIX vs PowerShell) |
| `inherit.rs` | Inherit user-level `~/.claude` resources (`agents`/`commands`/`skills`/`output-styles`) into each account dir: pure planning (`plan_links`/`has_conflict`/`resolve_subdir`) + edge I/O (`ensure_inherited`, per-entry symlink with Windows copy fallback). Stale inherited links are pruned by pure `plan_prune`: Skip removes every symlink pointing into `<source>/<sub>`, Merge only those whose source entry vanished, and a deleted `<source>/<sub>` prunes every link into it; regular files and foreign symlinks are never touched, the link (not its target) is removed, and `remove_link` re-checks it is still a symlink right before unlinking. `OS_JUNK` (`.DS_Store`, `Thumbs.db`, `desktop.ini`; case-insensitive via `is_os_junk`) is excluded from `plan_links`, `subdir_status`, `has_conflict` and the prune keep-set, so existing junk links get cleaned. Also one-shot **seeding** of root-level files (`SEEDED_FILES` = `settings.json`): `plan_file_seeds` (pure) + `ensure_seeded` (real copy, never symlink; existing dest file always wins). `plugins` is intentionally excluded (enablement lives in per-account `.claude.json`) |
| `adapters.rs` | Pluggable terminal adapters: declarative templates with `{{script}}`/`{{cwd}}`, `builtin_adapters` (per-OS via `#[cfg]`), `render_args`, `spawn` |
| `session.rs` | Session-expiry detection: `SessionStatus` (`LoggedOut { last_email }`/`LoggedIn`/`Expired`), pure `evaluate_credentials` + `keychain_service` (`Claude Code-credentials-<sha256(config_dir)[0..8]>`), credential reads from `.credentials.json` or macOS Keychain, `ensure_session_usable` launch gate (refuses `LoggedOut` and `Expired`), pure `status_from` + `security_item_not_found` (macOS `security` exit 44 / "could not be found" → `Absent`), 30s verdict cache (`Absent` never cached; `invalidate_verdict` on tray auth actions drops the entry and opens a 120s no-cache grace window so the terminal's outcome shows on the next hover). Absent credentials → `LoggedOut` even if `.claude.json` still records an email; *unreadable* credentials stay `LoggedIn` (never lock out on a denied Keychain prompt) |
| `cli.rs` | Pure decision logic for the `cms` CLI: `parse_cli_args` → `CliCommand`, fuzzy `match_account` (`AccountMatchError`: none/ambiguous), `launch_plan` |
| `usage.rs` | Per-account token usage from `<config_dir>/projects/**/*.jsonl` over rolling windows (5h session / 7d week): pure aggregation (`TokenTotals`, `UsageSummary`, window starts, formatting) + thin file-walking edge `account_usage`. Never reads the default `~/.claude` |
| `bin/cms.rs` | `cms` binary: edge I/O only (config via `Config::try_load` + `standalone_config_file_path`, session gate, inherit, exec `claude` inline); logic lives in `cli.rs` |
| `commands.rs` | `#[tauri::command]`s: `get_config` (returns `Result`: a corrupt config surfaces as an error in Preferences)/`save_config`/`list_terminals`, `get_inherit_status`/`set_inherit_decision`, `launch_session`/`open_session`/`login_account`/`logout_account`/`relogin_account`; on spawn failure it copies the manual command built by `launcher.rs` to the clipboard. All config access goes through the managed `ConfigStore` |
| `tray.rs` | `build_menu` (from config) + `build_tray` (icon + events) + `refresh_tray` (live rebuild via `tray_by_id` + `set_menu`); `parse_menu_id`/`MenuAction` |
| `lib.rs` | Builder wiring: plugins (single-instance first — a second launch focuses Preferences and exits; dialog, clipboard), manages the `ConfigStore`, `invoke_handler`, tray setup, window close-to-hide |

Frontend (`src/`): `api.ts` (typed `invoke` wrappers + types), `App.tsx` (Preferences), `App.css`.

**Boundaries:** `launcher` knows nothing about terminals; `adapters` know nothing about accounts; `config` is the single source of truth; `tray`/`commands` are thin glue.

**Menu-id contract:** `build_menu` emits ids `launch::<account>::<project>`, `session::<account>`, `login|logout|relogin::<account>`, `prefs`, `quit`; `parse_menu_id` must round-trip them exactly. `status::<account>` is a disabled item → `Unknown`.

## Invariants — do not break

- **Never write inside the default `~/.claude`.** The app may **read/list**
  `~/.claude/<sub>` to inherit user-level resources (`inherit.rs`), but every
  write — links, copies, config — lands in the per-account `~/.claude-<suffix>`
  dirs from config. `Project.account` and `Account.config_dir` flow through
  `expand_tilde`.
- **Config writes go through `ConfigStore::update`** (read-modify-write under one lock, atomic save). Never hold the lock across a blocking dialog or terminal spawn: do the I/O, then take the lock for the short update.
- **Shell-escape** every `config_dir`/`project_path` interpolated into scripts (POSIX `'\''`, PowerShell: double every single-quote delimiter, ASCII `'` **and** typographic `‘` `’` `‚` `‛`, via `powershell_single_quote_escape`). PowerShell paths go through `-LiteralPath`, and `.ps1` files are written with a UTF-8 BOM (`powershell.exe` 5.1 reads BOM-less scripts as ANSI). Adding a new script builder means adding escaping + a test.
- **Temp scripts**: atomic create, restrictive perms, unguessable name (use the `tempfile` path in `write_script`).
- **GUI PATH**: the app process lacks the user's shell `PATH` (no `~/.local/bin`), so `claude` is invoked **through the terminal adapter**, never via a bare `Command::new("claude")`.

## Conventions

- Code, identifiers, docs, commit messages, PRs: **English**.
- TDD: write the failing test first for pure logic. Naming: `test_should_X_when_Y`. Keep `cargo clippy -- -D warnings` clean (gate cross-OS dead code with targeted `#[cfg_attr(not(target_os = "..."), allow(dead_code))]`, never crate-wide).
- A repo hook blocks `git commit` without validation tooling; commit with `git commit --no-verify`.
- Tauri **v2** APIs only (`TrayIconBuilder`, `MenuBuilder`, `app.path()`), never v1.
- Config dirs use the fixed `~/.claude-` prefix + a free suffix (enforced in the Preferences UI).

## Spec & history

Design + plan live in `docs/superpowers/`. Manual verification steps in `docs/SMOKE-CHECKLIST.md`. User-facing changes are tracked in `CHANGELOG.md`.
