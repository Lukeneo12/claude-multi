# claude-multi

A cross-OS system-tray app (Tauri v2 + Rust + React/TypeScript) that lets you launch interactive [Claude Code](https://claude.ai/code) sessions under multiple accounts simultaneously — with zero risk of account bleed.

---

## What it does

Each account gets its own isolated `CLAUDE_CONFIG_DIR`. When you click a project in the tray, the app writes an ephemeral shell script, opens your configured terminal, and runs:

```sh
export CLAUDE_CONFIG_DIR=~/.claude-personal   # or ~/.claude-dino, etc.
cd /path/to/your/project
exec claude
```

The default `~/.claude` directory is **never touched**. You can run two (or more) accounts side by side in separate terminals — each authenticated independently, no re-auth required after the one-time login per account.

---

## CLAUDE_CONFIG_DIR-per-account model

| Concept | Details |
|---|---|
| Account | A label + a `CLAUDE_CONFIG_DIR` path (defaults: `~/.claude-personal`, `~/.claude-dino`) |
| Project | A label + a filesystem path; can appear under any account |
| One-time login | Run **Login…** once per account; OAuth opens in the browser, credentials are stored in that account's config dir |
| Isolation | Each session has its own `CLAUDE_CONFIG_DIR`; credentials, history, and settings never mix |

---

## Prerequisites

| Tool | Minimum | Check |
|---|---|---|
| `claude` CLI | any recent | `claude --version` |
| Node.js | 18 | `node --version` |
| Rust toolchain | stable | `cargo --version` |

If `cargo` is not on `PATH` in your shell, source it first:

```sh
. "$HOME/.cargo/env"
```

---

## Install

```sh
git clone <repo-url> claude-multi-session
cd claude-multi-session
npm install
```

---

## Run (development)

```sh
npm run tauri dev
```

This compiles the Rust backend and launches the app. A tray icon appears in the menu bar (macOS) or system tray (Linux/Windows).

---

## First-run flow

1. **Open Preferences**: click the tray icon → **Preferences…**
2. **Add projects**: enter a label and the absolute path to each local project directory.
3. **Login per account**: click the tray icon → *AccountName* → **Login…**
   - A terminal opens running `claude` with that account's `CLAUDE_CONFIG_DIR`.
   - Complete the OAuth flow in the browser once. The session is persisted in `~/.claude-personal` (or whichever dir the account uses).
   - Repeat for each account.
4. **Launch a session**: click the tray icon → *AccountName* → *ProjectName*.

---

## Terminal selection

In **Preferences…**, choose the terminal emulator to use for launching sessions.

| OS | Available |
|---|---|
| macOS | Terminal.app, iTerm2, Warp (verify — see below) |
| Linux | GNOME Terminal, Konsole |
| Windows | Windows Terminal, PowerShell |

**Warp note**: The Warp adapter (`open -a Warp <script>`) is included but requires manual verification that Warp executes the script rather than opening it as text. If Warp does not run the script, select Terminal.app or iTerm2 instead. Full Warp support (via Launch Configurations or URI scheme) is a planned follow-up. See `docs/SMOKE-CHECKLIST.md` for the verification steps.

---

## Configuration

App config is stored as `config.json` in the Tauri app-config directory:

- **macOS**: `~/Library/Application Support/com.lucasdonadio.claude-multi/config.json`
- **Linux**: `~/.config/com.lucasdonadio.claude-multi/config.json`
- **Windows**: `%APPDATA%\com.lucasdonadio.claude-multi\config.json`

You can edit it directly; changes take effect after a restart.

---

## Terminal spawn failure / clipboard fallback

If the configured terminal cannot be opened, the app copies the manual launch command to the clipboard:

```sh
CLAUDE_CONFIG_DIR='<config_dir>' sh -c "cd '<project_path>' && exec claude"
```

Paste it into any terminal to start the session manually.

---

## Known v1 caveats

- **Restart to refresh tray**: After editing accounts or projects in Preferences, the tray menu does not update automatically. Quit and relaunch the app to apply changes.
- **Warp adapter unverified**: See "Warp note" above and `docs/SMOKE-CHECKLIST.md`.

---

## Development

```sh
# Run all Rust tests
cd src-tauri && cargo test

# Lint (no warnings allowed)
cd src-tauri && cargo clippy --all-targets -- -D warnings
```

All 18 unit tests cover the config round-trip, launcher script generation (POSIX + PowerShell escaping), terminal adapter substitution, tray menu-id parsing, and the clipboard fallback.
