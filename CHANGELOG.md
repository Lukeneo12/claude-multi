# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project aims to
adhere to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## Unreleased

### Added

- Accounts now inherit your user-level `~/.claude` agents, commands, skills, and
  output-styles into each isolated session. If an account already has its own
  files in one of these, you're asked once whether to merge or keep it isolated.
  (Plugins are not inherited: plugin enablement is stored per-account, so a
  shared `plugins/` would show up disabled — enable plugins per account instead.)
- **Inheritance panel** in Preferences: pick an account to see, per subdir
  (`agents`/`commands`/`skills`/`output-styles`), whether it's `inherited`,
  `skipped`, in `conflict`, or has nothing to inherit (`none`) — with a
  **Merge / Skip** toggle to set the decision for any subdir without launching a
  session. The launch-time native dialog stays as the fallback for the still
  undecided case.

### Changed

- Inheritance decisions are now **sticky**: a `Skip` you choose is kept on later
  launches even if the account's own files for that subdir are later removed.
  Previously the choice was re-prompted once the conflict disappeared; manage it
  from the new Inheritance panel instead.

## [0.1.0] — 2026-06-24

Initial version: a cross-OS tray app to run Claude Code under multiple accounts,
each isolated in its own `CLAUDE_CONFIG_DIR`.

### Added

- **Multi-account isolation** via one `CLAUDE_CONFIG_DIR` per account; accounts run
  in parallel with no logout/login. The default `~/.claude` is never touched.
- **System tray** (Tauri v2, native): one submenu per account listing its projects,
  a **New session** item (project-less), and login/account actions.
- **Account-aware login state**: each account shows its logged-in email (read from
  `<config_dir>/.claude.json`) or **Login…** when signed out.
- **Log out** and **Re-login…** actions per logged-in account (`claude auth logout` /
  `claude auth login`, run through the terminal).
- **Projects belong to one account** and appear only under that account in the tray.
- **Preferences window** (React): manage accounts (add/remove, label, `~/.claude-`
  suffix), projects (label, path, owning account), and the launch terminal — with a
  **folder picker** to select a project directory and a polished, light/dark UI.
- **Pluggable terminal adapters** (macOS Terminal/iTerm/Warp, Linux gnome-terminal/
  konsole, Windows Windows Terminal/PowerShell) selectable in Preferences.
- **Live tray refresh**: the menu rebuilds on save and on hover — no restart required
  (hover-refresh on macOS/Windows; save-refresh everywhere).
- **Clipboard fallback**: if the terminal can't be opened, the equivalent manual
  command is copied to the clipboard and surfaced via a dialog.

### Fixed

- Tray icon now shows its menu (removed a duplicate menuless tray declared in
  `tauri.conf.json`; the programmatic tray sets its own icon + left-click menu).
- Preferences window hides on close instead of being destroyed, so it can reopen.
- Tray action failures are surfaced via a dialog instead of failing silently.
- Projects with an empty/invalid account are healed on load and shown with an explicit
  placeholder in the account dropdown (no more silent "first option" mis-selection).
- Save-status message clears on window focus and auto-dismisses after a few seconds.
- Opening the frontend outside Tauri shows guidance instead of hanging on "Loading…".

### Security

- Launch/login/logout scripts shell-escape the config dir and project path
  (POSIX `'\''`, PowerShell `''`).
- Ephemeral scripts are created atomically with restrictive permissions and
  unguessable names (`tempfile`).

[0.1.0]: https://example.com/claude-multi/releases/tag/v0.1.0
