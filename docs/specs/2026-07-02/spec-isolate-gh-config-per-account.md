# Spec: Isolate GitHub CLI (gh) configuration per account

| Field | Value |
|-------|-------|
| **Date** | 2026-07-02 |
| **Author** | Lukeneo12 |
| **Status** | Draft |
| **Type** | Feature |
| **Related PRD** | N/A |

---

## 1. Context / Problem

claude-multi isolates each Claude Code account in its own `CLAUDE_CONFIG_DIR`,
but the GitHub CLI keeps its state (active account, OAuth tokens) in a single
global location (`~/.config/gh/hosts.yml`). When several sessions run
concurrently under different accounts — the app's core use case — a
`gh auth switch` executed in one session silently changes the active GitHub
identity for **every** terminal on the machine.

Motivating incident (2026-07-02): during a single working session the global
gh active account flipped four times between a personal and a work account,
causing repeated `git push` failures with 403 (the push authenticated as the
work account against a personal repo). The failure mode is silent: nothing in
the affected session indicates the identity changed underneath it.

Desired state: each claude-multi account gets its own isolated gh
configuration, so gh identity follows the account of the session, exactly as
Claude Code identity already does.

## 2. Goals / Non-goals

### Goals

- Every terminal session launched from the tray (launch project, New session,
  Login, Logout, Re-login) exports `GH_CONFIG_DIR` pointing to a directory
  private to the launching account.
- A `gh auth switch` (or any gh state change) inside one account's session has
  no effect on other accounts' sessions or on the machine's global gh config.
- The per-account gh state lives inside the account's existing config dir, so
  removing the account directory removes all of its state.
- Shell-escaping and unit-test coverage for the new interpolation in every
  script builder, POSIX and PowerShell (project invariant).

### Non-goals

- A Preferences UI panel showing per-account gh auth status.
- Migrating, copying, or inheriting the user's existing global gh
  configuration into account dirs (see Key decisions).
- Isolating other tools with global state (`git config --global`, AWS
  profiles); the implementation may keep the door open internally, but nothing
  is exposed.
- A per-account opt-out toggle (v1 is always-on).
- Writing anywhere outside the per-account config dirs; the global
  `~/.config/gh` is never read or modified.

## 3. Acceptance Criteria

- [ ] AC1: Given any of the four script builders (`build_script`,
      `build_login_script`, `build_logout_script`, `build_relogin_script`),
      when a script is built for an account with config dir `<dir>`, then the
      script exports `GH_CONFIG_DIR='<dir>/gh'` (POSIX) or
      `$env:GH_CONFIG_DIR = '<dir>\gh'` (PowerShell) before invoking any
      command.
- [ ] AC2: Given a `config_dir` containing a single quote, when the script is
      built, then the `GH_CONFIG_DIR` value is escaped with the same rules
      already applied to `CLAUDE_CONFIG_DIR` (`'\''` POSIX, `''` PowerShell).
- [ ] AC3: Given two concurrent sessions for accounts A and B, when
      `gh auth switch` runs in A's session, then `gh api user` in B's session
      and in a plain (non claude-multi) terminal still resolve their previous
      identities. (Manual verification — added to `docs/SMOKE-CHECKLIST.md`.)
- [ ] AC4: The app never creates or writes the `<dir>/gh` directory itself;
      gh creates it on first use. No code path touches the global
      `~/.config/gh`.
- [ ] AC5: `cargo test` passes with new tests per builder per script kind
      covering AC1–AC2; `cargo clippy --all-targets -- -D warnings` and
      `cargo fmt --check` stay clean.
- [ ] AC6: README and CHANGELOG document the behavior change, including the
      one-time `gh auth login` per account.

## 4. Approach

All changes land in `src-tauri/src/launcher.rs`; no other module knows about
gh (same boundary discipline as `CLAUDE_CONFIG_DIR`: `adapters` and `tray`
stay untouched, `commands.rs` keeps calling the same builder signatures).

Each builder currently emits one `export CLAUDE_CONFIG_DIR=...` line. They
will additionally emit a `GH_CONFIG_DIR` line derived from the same
`config_dir` argument:

```sh
#!/bin/sh
export CLAUDE_CONFIG_DIR='/home/u/.claude-dino'
export GH_CONFIG_DIR='/home/u/.claude-dino/gh'
cd '/repo/app' || exit 1
exec claude
```

```powershell
$env:CLAUDE_CONFIG_DIR = 'C:\Users\u\.claude-dino'
$env:GH_CONFIG_DIR = 'C:\Users\u\.claude-dino\gh'
Set-Location 'C:\repo\app'
claude
```

The `<config_dir>/gh` path is composed once by a small pure helper (e.g.
`gh_config_dir(config_dir) -> String`) using the platform path separator per
`ScriptKind`, then escaped through the existing quote-escaping helpers. To
keep the next tool cheap without building a framework, the env lines can be
generated from a private const list of `(ENV_VAR, subdir)` pairs — currently
one entry — but this stays an internal detail of `launcher.rs`.

"New session" needs no dedicated work: `open_session` reuses
`build_login_script`, so covering the four builders covers all five tray
actions.

### Key decisions

- **gh only, hardcoded:** no generic per-account env-var mechanism (UI,
  validation, arbitrary-key escaping) for a need that doesn't exist yet.
  Trade-off: the next tool requires touching code — acceptable, it's one
  const-list entry.
- **`<config_dir>/gh` subdir:** account state stays self-contained and is
  removed together with the account dir; no new dotfiles in `$HOME`.
  Trade-off: a user hand-inspecting `~/.claude-<suffix>` sees a non-Claude
  subdir there; mitigated by README documentation.
- **Empty bootstrap, one-time `gh auth login` per account:** deliberately NOT
  copying the global gh config. Copying would duplicate OAuth tokens of *all*
  GitHub accounts into every isolated dir — multiplying secrets on disk and
  re-importing exactly the identity ambiguity this feature eliminates.
  Trade-off: one interactive login per account, once.
- **Always-on, no toggle:** consistent with the app's isolation promise; less
  config surface. A toggle can be added later if a real "I want the global gh"
  use case appears.

### Alternatives considered

- **Option A — generic per-account env-var map in `Config`:** rejected;
  adds UI + validation + escaping surface for hypothetical needs (YAGNI).
- **Option B — inherit global gh config on first launch (inherit.rs style):**
  rejected; copies credentials for every GitHub account into each account dir
  (worse security posture) and preserves the wrong-identity ambiguity.
- **Option C — wrap gh with a per-account shim script on PATH:** rejected;
  fragile (PATH ordering inside GUI-spawned terminals is exactly what the
  project avoids relying on) and more moving parts than one env var.

## 5. Risks / Rollback

### Risks

| Risk | Probability | Impact | Mitigation |
|------|-------------|--------|------------|
| User surprise: `gh` appears logged-out in sessions until the one-time per-account login | High | Low | Document in README + CHANGELOG; smoke-checklist entry; error message from gh itself is self-explanatory (`gh auth login`) |
| Tooling that reads gh's global config path directly (not via `gh`) ignores `GH_CONFIG_DIR` | Low | Low | Out of scope; such tools bypass gh's own contract |
| Older gh versions (< 2.x) not honoring `GH_CONFIG_DIR` | Low | Low | `GH_CONFIG_DIR` is supported since gh 1.9 (2021); no action |
| A future claude-code feature also setting `GH_CONFIG_DIR` inside sessions | Low | Med | Script sets it before `exec claude`; child processes inherit ours unless explicitly overridden — revisit if it ever happens |

### Rollback plan

Revert the PR (single commit range touching `launcher.rs`, its tests, README,
CHANGELOG). Sessions launched afterwards fall back to the global gh config.
Per-account `gh/` subdirs left behind are inert data; users can delete them
manually (documented in the PR description). No config-file format change, no
migration to undo.

## 6. Open questions

- None.

---

*Spec generated with `/spec` skill. Update this file if the approach changes during implementation.*
