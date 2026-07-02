---
name: project-claude-multi-overview
description: What claude-multi is and where its test-heavy logic lives, for QA review context
metadata:
  type: project
---

claude-multi is a Tauri v2 + Rust tray app that launches Claude Code sessions
under multiple accounts, each isolated in its own `CLAUDE_CONFIG_DIR`
(`~/.claude-<suffix>`). Frontend is React/TS (Preferences window only).

**Why this matters for QA:** `src-tauri/src/launcher.rs` is the most
test-relevant module — it holds pure script-builder functions
(`build_script`/`build_login_script`/`build_logout_script`/`build_relogin_script`)
that interpolate user-controlled strings (`config_dir`, `project_path`) into
shell/PowerShell scripts. Every new interpolated value must be shell-escaped
(POSIX `'\''`, PowerShell `''`) and have unit tests per builder per
`ScriptKind`. This is the module most likely to need new tests when a
feature adds another env var or interpolated value (see
[[project-gh-isolation-feature]] for the first example of this pattern:
`GH_CONFIG_DIR` added via a `PER_ACCOUNT_ENV_VARS` const list).

**How to apply:** When reviewing new launcher.rs features, check test
coverage per builder (4) per script kind (2) = 8 combinations minimum, plus
escaping tests and ordering tests (new env vars must be set before the first
command that depends on them, for both POSIX and PowerShell — see
[[feedback-launcher-test-substring-gotcha]] for a pitfall when asserting
ordering with string search).

Repo invariants worth re-checking on every launcher.rs change: never write
inside default `~/.claude`, shell-escape every interpolated path, atomic
temp-script creation with restrictive perms (`tempfile` in `write_script`),
GUI process lacks user PATH so `claude`/`gh` are only ever invoked through
scripts run by a terminal adapter (no bare `Command::new(...)`).
