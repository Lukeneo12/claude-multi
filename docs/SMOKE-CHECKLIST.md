# Smoke Checklist — claude-multi

Manual verification steps. Run after every significant change or before a release.
Automated tests (`cargo test`, `cargo clippy --all-targets -- -D warnings`,
`npm run build`) cover the pure logic; this checklist covers the GUI/terminal/OAuth
flows that can't be tested headlessly.

---

## Prerequisites (all OS)

- `claude` CLI on PATH with `auth` subcommands (`claude auth status`)
- Node.js ≥ 18 (`node --version`)
- Rust toolchain with `cargo` on PATH (`cargo --version`; else `. "$HOME/.cargo/env"`)
- `npm install` completed at project root

---

## macOS — full flow

### 1. Build & launch
```sh
npm run tauri dev
```
**PASS**: app compiles; a tray icon appears in the menu bar; **no window** opens on
startup; clicking the icon opens a menu (not nothing — regression guard for the
duplicate-tray bug).

### 2. Web-fallback guard
Open the Vite dev URL (e.g. `http://localhost:1420`) in a browser.
**PASS**: shows the guidance message ("Open this window from the claude-multi tray
icon…"), not a stuck "Loading…" or console `invoke` errors.

### 3. Accounts in Preferences
Tray → **Preferences…** → **Accounts**.
- **PASS**: exactly one default account, **Personal** with config dir `~/.claude-`+`personal`.
- Click **+ Add account**; set label `DinoCloud` and suffix `dino` (prefix `~/.claude-`
  is fixed). Remove it with ✕ and re-add to confirm add/remove work.

### 4. Projects + folder picker
**Projects** section → **+ Add project** → **Browse…**.
- **PASS**: a native folder picker opens; selecting a folder fills the path and
  auto-fills the label with the folder name.
- **PASS**: the account dropdown defaults to the first account; assigning the project
  to an account works. (A project with no valid account shows a **"— account —"**
  placeholder, never a misleading first-option selection.)

### 5. Save → live refresh (no restart)
Click **Save**.
- **PASS**: status reads "Saved — the tray menu was updated."; open the tray menu and
  the new project appears **under its assigned account only** (not under every account).
- **PASS**: close Preferences and reopen it — the "Saved" message is gone (cleared on
  focus / auto-dismiss).

### 6. Login per account (OAuth — one time each)
Before: `ls -la ~/.claude` (note timestamps; if absent it must stay absent).
Tray → **Personal** → **Login…**.
- **PASS**: the configured terminal opens running `claude` with
  `CLAUDE_CONFIG_DIR=~/.claude-personal`; complete OAuth in the browser.
- Repeat for **DinoCloud** (→ `~/.claude-dino`).

### 7. Logged-in email + live hover refresh
After logging in, hover the tray icon and open the account submenu.
- **PASS**: instead of **Login…**, the submenu shows **✓ <email>** (disabled) plus
  **Re-login…** and **Log out** — without restarting the app (hover-refresh).

### 8. New session (project-less)
Tray → *Account* → **New session**.
- **PASS**: a terminal opens running `claude` under that account's `CLAUDE_CONFIG_DIR`
  with no project `cd` (a plain session in your home dir).

### 9. Launch a project under each account
Tray → **Personal** → *[project]*, then **DinoCloud** → *[its project]*.
- **PASS**: terminal opens; `echo $CLAUDE_CONFIG_DIR` matches the account; `pwd` is the
  project path.

### 10. Two simultaneous sessions, no re-auth
With both terminals open: **PASS** — both are already authenticated; neither prompts OAuth.

### 11. Log out / Re-login
Tray → *Account* → **Log out**.
- **PASS**: a terminal runs `claude auth logout` ("Successfully logged out"); hover +
  reopen the menu → that account now shows **Login…** again.
- **Re-login…**: runs `claude auth logout` then `claude auth login` (browser OAuth).

### 12. `~/.claude` is untouched
```sh
ls -la ~/.claude
```
**PASS**: timestamps/contents of the default `~/.claude` are unchanged by any of the
above; if it didn't exist, it still doesn't.

### 13. Clipboard fallback (invalid terminal)
Set `"terminal": "does-not-exist"` in `config.json`
(`~/Library/Application Support/com.lucasdonadio.claude-multi/config.json`) and launch
a project from the tray.
- **PASS**: a dialog reports the terminal couldn't be opened and the command was copied
  to the clipboard. Paste (`Cmd+V`) → contains:
  ```
  CLAUDE_CONFIG_DIR='<config_dir>' sh -c "cd '<project_path>' && exec claude"
  ```
- Restore `terminal` to `terminal` (valid macOS default) afterward.

### 14. Warp adapter
See "Warp Adapter Decision" below.

---

## Linux (Ubuntu / GNOME) — deltas

- Tray icon may require `gnome-shell-extension-appindicator`.
- Default terminal `gnome-terminal`; also test Konsole by switching in Preferences.
- Run steps 3–13 (skip step 2 web-fallback wording differences are fine).
- **Hover-refresh (step 7/11) does not apply on Linux** (no tray hover events) — verify
  the **save-refresh** path instead: after a config change, Save updates the menu.
- Clipboard paste: `Ctrl+V`.

## Windows — deltas

- Tray icon in the taskbar notification area.
- Config dirs: `%USERPROFILE%\.claude-personal`, etc.; default terminal Windows Terminal
  (`wt`), scripts are `.ps1`. Verify `$env:CLAUDE_CONFIG_DIR` in the PowerShell session.
- Hover-refresh applies (Windows emits tray events).
- Clipboard fallback contains the PowerShell-shaped command.

---

## Warp Adapter Decision (macOS)

**Status: Unverified — requires an interactive GUI session.**

Warp is installed at `/Applications/Warp.app`. The adapter command is
`open -a Warp {{script}}`. Whether Warp executes the script (vs. opening it as text)
is application-specific and can't be confirmed headlessly.

**Verify**: set Terminal to "Warp (verify)", launch a project.
- **PASS**: Warp opens and `claude` starts in the right project dir with
  `CLAUDE_CONFIG_DIR` set.
- **FAIL**: Warp opens but doesn't run the script → use Terminal.app/iTerm2. Native Warp
  support (Launch Configurations / `warp://`) is a follow-up.

The label stays **"Warp (verify)"** until a human confirms pass/fail on a GUI session.

---

## Inherited Resources

- [ ] **Inherited resources:** With user-level agents/commands in `~/.claude`,
      launch a session for an account whose dir lacks them → the account dir
      gains `agents/`, `commands/`, `skills/`, `output-styles/`, `plugins/` with
      symlinks, and the agents/commands appear inside the session.
- [ ] **Conflict prompt:** For an account that already has its own `agents/`,
      launching prompts once (Merge/Skip); the choice persists (no prompt on the
      next launch) and is saved in `config.json` under `inherit_overrides`.
- [ ] **Plugins sanity:** Confirm a session with an inherited `plugins/` loads
      without errors (validates the plugins caveat from the spec).

---

## Known caveats

- **Warp adapter**: unverified (above).
- **Linux hover-refresh**: not available (no tray hover events); save-refresh works.
- **Distribution**: v1 runs from a local build; no code signing / notarization / installers yet.
