# claude-multi

A cross-OS system-tray app (Tauri v2 + Rust + React/TypeScript) that launches interactive [Claude Code](https://claude.ai/code) sessions under multiple accounts — without ever logging out and back in.

Run your personal account and a corporate account side by side: each is isolated in its own `CLAUDE_CONFIG_DIR`, so credentials, history, and settings never mix, and the default `~/.claude` is never touched.

---

## How it works

Each account maps to a dedicated `CLAUDE_CONFIG_DIR` under `~/.claude-<suffix>`. When you pick an action in the tray, the app writes an ephemeral, permission-restricted shell script, opens your configured terminal, and runs `claude` with that account's config dir — for example:

```sh
export CLAUDE_CONFIG_DIR=~/.claude-personal
cd /path/to/your/project
exec claude
```

On macOS, Claude Code stores each config dir's credentials in a separate Keychain entry; on Linux/Windows, in a `.credentials.json` inside the dir. Either way, accounts stay fully isolated and can run in parallel with no re-auth after the one-time login.

---

## Concepts

| Concept | Details |
|---|---|
| **Account** | A label + a `CLAUDE_CONFIG_DIR` (always `~/.claude-<suffix>`). The default config seeds one account, **Personal** (`~/.claude-personal`). Add more in Preferences. |
| **Project** | A label + a folder path + the **one account** it belongs to. A project appears only under its account in the tray. |
| **Session** | An interactive `claude` run under an account — either in a project (**project** item) or outside any project (**New session**). |

---

## The tray menu

Clicking the tray icon shows one submenu per account:

```
Personal ▸
  New session                 ← claude under this account, no project
  cozify-backend              ← this account's projects
  ──────────────
  ✓ you@example.com           ← logged-in email (or "Login…" if not)
  Re-login…                   ← claude auth logout + login (switch account)
  Log out                     ← claude auth logout
──────────────
Preferences…
Quit
```

- **Logged-in state** is read from `<config_dir>/.claude.json` (`oauthAccount.emailAddress`). When logged in, the email is shown and the account offers **Re-login…** / **Log out**; otherwise it shows **Login…**.
- The menu refreshes **live**: after you save in Preferences, and on hover (so logins/logouts done in a terminal show up the next time you open the menu). No restart needed (hover-refresh is macOS/Windows only — Linux doesn't emit tray hover events, but save-refresh works everywhere).

---

## Prerequisites

| Tool | Minimum | Check |
|---|---|---|
| `claude` CLI | recent (`auth` subcommands) | `claude auth status` |
| Node.js | 18 | `node --version` |
| Rust toolchain | stable | `cargo --version` |

If `cargo` is not on `PATH`, source it first: `. "$HOME/.cargo/env"`

---

## Install & run

```sh
git clone <repo-url> claude-multi-session
cd claude-multi-session
npm install
npm run tauri dev
```

This compiles the Rust backend and launches the app. A tray icon appears in the menu bar (macOS) / system tray (Windows/Linux); the app has no main window — open **Preferences…** from the tray.

> The app uses Tauri's IPC bridge, so the Preferences window only works inside the Tauri webview. Opening the Vite dev URL in a plain browser shows a guidance message instead.

---

## First-run flow

1. **Preferences…** → under **Accounts**, keep *Personal* and/or add accounts (label + a `~/.claude-<suffix>`).
2. Add **Projects**: click **Browse…** to pick a folder (the label auto-fills), then assign each project to an account. **Save.**
3. **Login per account**: tray → *Account* → **Login…** → a terminal opens running `claude`; complete the browser OAuth once.
4. **Launch**: tray → *Account* → *Project* (or **New session** for a project-less session).

---

## Terminal selection

In **Preferences…**, choose which terminal opens for sessions and logins.

| OS | Available |
|---|---|
| macOS | Terminal.app, iTerm2, Warp (verify — see below) |
| Linux | GNOME Terminal, Konsole |
| Windows | Windows Terminal, PowerShell |

**Warp note**: the Warp adapter (`open -a Warp <script>`) is included but unverified — confirm Warp runs the script rather than opening it as text; otherwise use Terminal.app/iTerm2. See `docs/SMOKE-CHECKLIST.md`.

---

## Configuration

Config is stored as `config.json` in the Tauri app-config directory:

- **macOS**: `~/Library/Application Support/com.lucasdonadio.claude-multi/config.json`
- **Linux**: `~/.config/com.lucasdonadio.claude-multi/config.json`
- **Windows**: `%APPDATA%\com.lucasdonadio.claude-multi\config.json`

You can edit it directly; the tray reflects changes on the next save (from Preferences) or hover.

If the configured terminal can't be opened, the app copies a manual command to the clipboard so you can paste and run it yourself.

---

## Security model

- The app **only** reads/writes the per-account `~/.claude-<suffix>` dirs you configure — **never** the default `~/.claude`.
- Launch scripts shell-escape the config dir and project path (POSIX `'\''`, PowerShell `''`).
- Temp scripts are created atomically with restrictive permissions and unguessable names (`tempfile`).

---

## Development

```sh
cd src-tauri
cargo test                              # 27 unit tests
cargo clippy --all-targets -- -D warnings
```

```sh
npm run build                           # typecheck + build the frontend
```

Tests cover config defaults/round-trip and account-email lookup, launcher script generation (POSIX + PowerShell escaping), terminal-adapter substitution, tray menu-id parsing, and the clipboard fallbacks. See `CLAUDE.md` for architecture and conventions.

---

## Build a distributable

```sh
npm run tauri build
```

This produces a release bundle **for the OS you build on** (Tauri can't cross-compile native installers), under `src-tauri/target/release/bundle/`:

| OS | Output |
|---|---|
| macOS | `claude-multi.app` + `claude-multi_<ver>_<arch>.dmg` |
| Linux | `.deb` / `.rpm` / `.AppImage` |
| Windows | `.msi` / `.exe` (NSIS) |

So **each platform builds its own** — clone the repo and run the command on macOS, Linux, and Windows respectively (or wire up CI with per-OS runners later).

**Unsigned builds**: bundles are ad-hoc signed (no Developer ID / notarization), so other machines' Gatekeeper/SmartScreen will warn. On macOS the recipient can right-click → **Open** once, or run `xattr -dr com.apple.quarantine /Applications/claude-multi.app`. Frictionless distribution needs platform code-signing (e.g. Apple Developer ID + notarization via `bundle.macOS.signingIdentity` and the `APPLE_*` env vars) — not set up here.

Useful flags: `--bundles app|dmg|deb|…` to pick targets; `--target universal-apple-darwin` for a universal macOS binary (`rustup target add x86_64-apple-darwin` first).

---

## License

[MIT](LICENSE) © Lucas Donadio.

---

## Known caveats

- **Warp adapter unverified** (see above).
- **Linux hover-refresh**: the tray doesn't refresh on hover on Linux (no tray hover events); it still refreshes on save.
- **Distribution**: builds are unsigned (see "Build a distributable"); each OS builds its own bundle.
