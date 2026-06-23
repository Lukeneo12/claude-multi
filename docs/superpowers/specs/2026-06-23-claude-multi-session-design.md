# Design Spec — `claude-multi`: Cross-OS Multi-Account Launcher for Claude Code

- **Date:** 2026-06-23
- **Status:** Approved (design)
- **Author:** Lucas Donadio (with Claude Code)

## Context / Problem

The user runs Claude Code under two different accounts:

- **Personal** — a Max subscription, logged in via Google (Gmail).
- **Corporate (DinoCloud)** — a subscription seat, logged in via Google SSO.

Both authenticate through the same interactive browser OAuth flow. Today, switching
between them means `/logout` + re-login, which is slow and disruptive. The goal is to
run tasks under either account **without ever logging out/in**, and to package this as a
**cross-OS product** that can be shared.

### Key technical finding (verified)

Claude Code isolates account state by `CLAUDE_CONFIG_DIR`:

- **macOS:** credentials live in the macOS Keychain under a service name derived from a
  hash of the config dir (verified empirically: entries like
  `Claude Code-credentials-08db5d00`). Distinct config dirs ⇒ distinct Keychain entries ⇒
  **no collision**.
- **Linux/Windows:** credentials live in `.credentials.json` **inside** the config dir,
  so they are isolated by directory naturally (even cleaner).

Therefore one `CLAUDE_CONFIG_DIR` per account is the universal, cross-OS abstraction for
account isolation. No API keys or Bedrock involved — both accounts are subscription OAuth.

(Verified against Claude Code 2.1.186 and the official auth/setup/headless docs.)

## Goals / Non-goals

### Goals (v1)

- Run **interactive** Claude Code sessions under either account with no logout/login.
- Cross-OS **system tray** app (macOS menu bar, Windows tray, Linux tray) built with Tauri.
- Pick **account → project → launch**; the app opens the user's terminal with the correct
  `CLAUDE_CONFIG_DIR` and `claude` running in the project directory.
- One-time **Login** action per account (browser OAuth once).
- **Pluggable terminal adapters** (declarative templates) so the launch terminal is
  configurable and new terminals can be added without recompiling.
- Preferences UI to manage accounts, projects, and the selected terminal.
- Persist configuration in a single config file.
- Purely additive: never modify the existing default `~/.claude`.

### Non-goals (deferred to later iterations)

- Headless task runner (`claude -p`, fire-and-forget).
- Session monitoring / cost / usage dashboards.
- Auto-detecting the account from the working directory.
- Auto-update and per-OS signing / notarization / distribution.
- Multi-user / team management.

## Acceptance Criteria

1. From a fresh setup, the user can register two accounts mapped to two **symmetric**
   config dirs (`~/.claude-personal`, `~/.claude-dino`) and complete a one-time login in
   each via the app's **Login** action.
2. Clicking *Account ▸ Project* opens the configured terminal with `CLAUDE_CONFIG_DIR`
   set to that account's dir, `cd` into the project path, and `claude` running.
3. Two sessions under different accounts can run **simultaneously** without either
   needing to re-authenticate.
4. The launch terminal is selectable in Preferences; switching it changes which terminal
   opens, with no code change.
5. The existing default `~/.claude` is never read or written by the app.
6. A failed launch (missing terminal / bad path) surfaces a clear error and offers a
   fallback (copy the command to clipboard).
7. Core logic (command/script builder, adapter template rendering, config load/save) is
   unit-tested to ~80% coverage with `test_should_X_when_Y` naming.

## Approach

### Components (single-responsibility units)

- **Config** — reads/writes `config.json` from Tauri's per-OS app-config dir. Validates
  schema; falls back to defaults on invalid input. Source of truth for accounts,
  projects, and selected terminal.
- **Launcher core** (pure) — given `(account, project)`, produces an ephemeral launch
  script:
  ```sh
  export CLAUDE_CONFIG_DIR=<account.configDir>
  cd <project.path>
  exec claude
  ```
  (`.sh` on macOS/Linux, `.ps1`/`.cmd` on Windows). Knows nothing about terminals.
- **Terminal adapters** (pluggable) — given `(terminalId, scriptPath, cwd)`, open
  terminal T running the script. Declarative templates with placeholders
  (`{{script}}`, `{{cwd}}`); knows nothing about accounts. The ephemeral-script approach
  sidesteps per-terminal env-passing quirks — adapters only need to "open T and run S".
  Built-in adapters: Warp + Terminal.app/iTerm2 (macOS), gnome-terminal/konsole (Linux),
  Windows Terminal/PowerShell (Windows). Users can add adapters via config.
- **Tray UI** (frontend, web) — menu listing accounts → projects, Login actions, and
  Preferences. A small Preferences window manages accounts/projects/terminal.

### Config shape (illustrative)

```json
{
  "terminal": "warp",
  "accounts": [
    { "id": "personal", "label": "Personal (Max)", "configDir": "~/.claude-personal" },
    { "id": "dino", "label": "DinoCloud", "configDir": "~/.claude-dino" }
  ],
  "projects": [
    { "id": "p1", "label": "claude-multi-session", "path": "~/claude-multi-session", "defaultAccount": "personal" }
  ]
}
```

### Account config-dir mapping (decided: option A)

Two **new, symmetric** config dirs (`~/.claude-personal`, `~/.claude-dino`). The app owns
account state fully; cleaner and portable. Cost: one login per dir on first run. The
default `~/.claude` is left untouched.

### Login flow

A per-account **Login** action launches a terminal with
`CLAUDE_CONFIG_DIR=<dir> claude`, triggering the browser OAuth once; the token is stored
in that account's Keychain entry (macOS) or `.credentials.json` (Linux/Windows).
Logged-in state is detected best-effort (presence of the credential).

### Error handling

- Missing config dir → offer to create it and prompt login.
- Terminal/adapter failure → clear tray error + fallback to copy command to clipboard.
- Missing project path → mark the project as broken in the menu.
- Invalid config file → load defaults, show error, never crash.

### Testing strategy

- **Rust core:** pure-function unit tests for the script builder and adapter template
  rendering (AAA, mock the actual spawn). Config round-trip + schema validation + defaults.
- **Frontend:** minimal logic tests for the menu model.
- **Manual:** per-OS smoke checklist (macOS now; Linux/Windows when iterating).
- Target ~80% coverage on the core layer.

## Risks / Rollback

| Risk | Mitigation |
|---|---|
| Warp's programmatic launch is less standard than `osascript` terminals | Ephemeral-script + early verification; fallback "copy command to clipboard" |
| macOS Keychain per-account isolation | Verified working (config-dir hash) — low risk |
| Claude Code changes `CLAUDE_CONFIG_DIR` behavior | Documented + verified; pinned to v2.1.186 in notes |
| Cross-OS distribution / code signing | Explicitly deferred to a later iteration |

**Rollback:** the tool is purely additive. It never touches `~/.claude`; it creates new
config dirs. Uninstall = delete the app + the config dirs. No impact on the current setup.
