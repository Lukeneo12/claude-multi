# Smoke Checklist — claude-multi

Manual verification steps per OS. Run these after every significant change or before a release.

---

## Warp Adapter Decision (macOS)

**Status: Unverified — requires interactive GUI session.**

Warp is installed at `/Applications/Warp.app` on the test machine.
The adapter command is `open -a Warp {{script}}`.

`open -a <App> <file>` on macOS opens the application with the file as an argument; whether Warp then executes the script as a shell command is application-specific behavior that cannot be confirmed in a headless/non-GUI agent context.

**Human verification step:**

1. Build and run the app (`npm run tauri dev`).
2. In Preferences, set Terminal to "Warp (verify)".
3. Trigger a project launch from the tray.
4. **PASS**: A Warp window opens and `claude` starts in the correct project directory with `CLAUDE_CONFIG_DIR` set.
5. **FAIL**: Warp opens but does not execute the script, or opens the script as a text file.

**Fallback if FAIL**: Select "Terminal.app" or "iTerm2" in Preferences. Adding native Warp support (via Warp Launch Configurations or the `warp://` URI scheme) is tracked as a follow-up.

The adapter label is intentionally kept as **"Warp (verify)"** until the above pass/fail is confirmed by a human on a GUI session.

---

## Prerequisites (all OS)

- `claude` CLI on PATH (run `claude --version` to verify)
- Node.js ≥ 18 (`node --version`)
- Rust toolchain with `cargo` on PATH (`cargo --version`)
- `npm install` completed at project root

---

## macOS

### 1. Build

```sh
npm run tauri dev
```

Expected: the app compiles, a system tray icon appears in the menu bar.

### 2. Register two accounts

1. Click the tray icon → **Preferences…**
2. In the Accounts section, verify two default accounts exist: "Personal (Max)" (`~/.claude-personal`) and "DinoCloud" (`~/.claude-dino`).
3. Add a test project (e.g., label "My Repo", path to any local directory).

### 3. Login per account (OAuth — one time each)

1. Click the tray icon → **Personal (Max)** → **Login…**
   - A terminal window opens running `claude` with `CLAUDE_CONFIG_DIR=~/.claude-personal`.
   - Complete OAuth in the browser.
   - Verify the terminal shows Claude authenticated.
2. Click the tray icon → **DinoCloud** → **Login…**
   - A second terminal opens with `CLAUDE_CONFIG_DIR=~/.claude-dino`.
   - Complete OAuth for the second account.

**PASS criteria**: each terminal uses a different config dir; neither touches `~/.claude`.

### 4. Confirm `~/.claude` is untouched

```sh
ls -la ~/.claude
```

Run before and after login. The timestamps on `~/.claude` (or its contents) must not change. If `~/.claude` does not exist, it must still not be created.

### 5. Launch a project under each account

1. Click tray → **Personal (Max)** → *[your test project]*
   - Terminal opens; verify `echo $CLAUDE_CONFIG_DIR` prints `~/.claude-personal` (expanded).
   - Verify `pwd` prints the project path.
2. Click tray → **DinoCloud** → *[your test project]*
   - Same project, different account.

### 6. Confirm two simultaneous sessions do not re-auth

With both terminals open side by side:
- Each session should already be authenticated from step 3.
- Neither session should prompt for OAuth.

**PASS**: Claude prompts in both terminals without re-authentication.

### 7. Clipboard fallback with invalid terminal

1. In Preferences, set Terminal to a non-existent ID (edit `config.json` directly, e.g., `"terminal": "does-not-exist"`).
2. Trigger a project launch from the tray.
3. **PASS**: An error toast/message appears saying the terminal could not be opened and the command was copied to the clipboard.
4. Paste clipboard (`Cmd+V` in any text field) — verify it contains:
   ```
   CLAUDE_CONFIG_DIR='<config_dir>' sh -c "cd '<project_path>' && exec claude"
   ```
5. Restore `terminal` to a valid value (e.g., `"terminal"`).

### 8. Warp adapter (if applicable)

See "Warp Adapter Decision" section at the top.

---

## Linux (Ubuntu / GNOME)

### 1. Build

```sh
npm run tauri dev
```

Expected: tray icon appears in system tray (may require `gnome-shell-extension-appindicator` on Ubuntu).

### 2–7. Same steps as macOS

Replace macOS-specific paths:
- `~/.claude-personal` and `~/.claude-dino` remain the same.
- Default terminal is `gnome-terminal`; verify step 2–7 with GNOME Terminal.
- Konsole: repeat steps 3–6 after switching terminal in Preferences.

### Clipboard fallback

Same as macOS step 7; paste with `Ctrl+V`.

---

## Windows

### 1. Build

```sh
npm run tauri dev
```

Expected: tray icon appears in the system tray (taskbar notification area).

### 2–7. Same steps as macOS

Replace macOS-specific paths:
- Config dirs: `%USERPROFILE%\.claude-personal` and `%USERPROFILE%\.claude-dino`.
- Default terminal is Windows Terminal (`wt`); scripts are `.ps1` (PowerShell).
- Verify `$env:CLAUDE_CONFIG_DIR` is set correctly in the PowerShell session.

### Clipboard fallback

Same flow; verify clipboard contains the PowerShell equivalent command.

---

## Known v1 Caveats

- **Restart to refresh tray**: After adding/editing accounts or projects in Preferences, the tray menu does not update automatically. Quit and relaunch the app (`npm run tauri dev` or the built binary) to see the changes.
- **Warp adapter**: Unverified; see top section. Use Terminal.app or iTerm2 for reliable macOS operation.
