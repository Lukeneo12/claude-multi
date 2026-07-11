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
| `paths.rs` | `expand_tilde`, `config_file_path` (Tauri app-config dir) |
| `config.rs` | `Account`/`Project`/`Config` models, defaults, JSON load/save, `Account::logged_in_email` (reads `<config_dir>/.claude.json`) |
| `launcher.rs` | Pure builders for ephemeral scripts (`build_script`/`build_login_script`/`build_logout_script`/`build_relogin_script`) + `write_script` (atomic, restrictive perms via `tempfile`) + quote-escaping helpers + `ScriptKind` |
| `inherit.rs` | Inherit user-level `~/.claude` resources (`agents`/`commands`/`skills`/`output-styles`) into each account dir: pure planning (`plan_links`/`has_conflict`/`resolve_subdir`) + edge I/O (`ensure_inherited`, per-entry symlink with Windows copy fallback). Also one-shot **seeding** of root-level files (`SEEDED_FILES` = `settings.json`): `plan_file_seeds` (pure) + `ensure_seeded` (real copy, never symlink; existing dest file always wins). `plugins` is intentionally excluded (enablement lives in per-account `.claude.json`) |
| `adapters.rs` | Pluggable terminal adapters: declarative templates with `{{script}}`/`{{cwd}}`, `builtin_adapters` (per-OS via `#[cfg]`), `render_args`, `spawn` |
| `commands.rs` | `#[tauri::command]`s: `get_config`/`save_config`/`list_terminals`, `launch_session`/`open_session`/`login_account`/`logout_account`/`relogin_account`, manual-command clipboard fallbacks |
| `tray.rs` | `build_menu` (from config) + `build_tray` (icon + events) + `refresh_tray` (live rebuild via `tray_by_id` + `set_menu`); `parse_menu_id`/`MenuAction` |
| `lib.rs` | Builder wiring: plugins (dialog, clipboard), `invoke_handler`, tray setup, window close-to-hide |

Frontend (`src/`): `api.ts` (typed `invoke` wrappers + types), `App.tsx` (Preferences), `App.css`.

**Boundaries:** `launcher` knows nothing about terminals; `adapters` know nothing about accounts; `config` is the single source of truth; `tray`/`commands` are thin glue.

**Menu-id contract:** `build_menu` emits ids `launch::<account>::<project>`, `session::<account>`, `login|logout|relogin::<account>`, `prefs`, `quit`; `parse_menu_id` must round-trip them exactly. `status::<account>` is a disabled item → `Unknown`.

## Invariants — do not break

- **Never write inside the default `~/.claude`.** The app may **read/list**
  `~/.claude/<sub>` to inherit user-level resources (`inherit.rs`), but every
  write — links, copies, config — lands in the per-account `~/.claude-<suffix>`
  dirs from config. `Project.account` and `Account.config_dir` flow through
  `expand_tilde`.
- **Shell-escape** every `config_dir`/`project_path` interpolated into scripts (POSIX `'\''`, PowerShell `''`). Adding a new script builder means adding escaping + a test.
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
