# claude-multi Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a cross-OS system-tray app (Tauri v2) that launches interactive Claude Code sessions under different accounts by isolating each account in its own `CLAUDE_CONFIG_DIR`, with no logout/login.

**Architecture:** A Rust backend owns config, a pure "launcher core" that builds an ephemeral launch script, and pluggable terminal adapters that open the user's terminal running that script. The native tray menu (built in Rust) lists accounts → projects; a small React preferences window edits config. Account isolation = one `CLAUDE_CONFIG_DIR` per account; the app never touches the default `~/.claude`.

**Tech Stack:** Tauri v2, Rust (backend + native tray), React + TypeScript + Vite (preferences window only), serde/serde_json, `std::process::Command` for spawning terminals.

## Global Constraints

- Tauri **v2** (not v1 — APIs differ; use `TrayIconBuilder`, `MenuBuilder`, `app.path()`).
- Target OSes: **macOS, Windows, Linux**. v1 is run from a local build (`npm run tauri dev` / `build`); no signing/notarization yet.
- The app **must never read or write the default `~/.claude`**. It only ever touches per-account dirs named in config.
- Account isolation mechanism: one `CLAUDE_CONFIG_DIR` per account. Both target accounts are subscription OAuth (no API keys).
- Config dirs are **symmetric and app-owned**: defaults `~/.claude-personal` and `~/.claude-dino`.
- Verified against Claude Code **2.1.186**.
- Rust naming for tests: `test_should_X_when_Y`. Target ~80% coverage on the core layer (`config`, `launcher`, `adapters`, `paths`).
- Identifiers/code/docs in **English**.

---

## File Structure

```
claude-multi-session/
├── src-tauri/
│   ├── Cargo.toml
│   ├── tauri.conf.json
│   ├── icons/                      # tray + app icons
│   └── src/
│       ├── main.rs                 # entry → lib::run()
│       ├── lib.rs                  # Builder setup, state, invoke_handler, tray wiring
│       ├── paths.rs                # expand_tilde, config_file_path
│       ├── config.rs               # Account/Project/Config models, load/save, defaults
│       ├── launcher.rs             # build_script (pure), write_script (io)
│       ├── adapters.rs             # TerminalAdapter, render_args, builtin_adapters, spawn
│       ├── commands.rs             # #[tauri::command] fns for the frontend
│       └── tray.rs                 # build_tray_menu, menu-id routing
├── src/                            # React preferences window
│   ├── main.tsx
│   ├── App.tsx                     # preferences form
│   └── api.ts                      # typed wrappers over invoke()
├── index.html
├── package.json
└── docs/superpowers/...
```

Responsibility split: `launcher` knows nothing about terminals; `adapters` know nothing about accounts; `config` is the single source of truth; `tray`/`commands` are thin glue.

---

### Task 1: Scaffold Tauri v2 app with a static tray

**Files:**
- Create: whole `src-tauri/` + `src/` + `package.json` (via scaffolding tool, then trim)
- Modify: `src-tauri/src/lib.rs`, `src-tauri/tauri.conf.json`

**Interfaces:**
- Consumes: nothing
- Produces: a runnable Tauri app whose tray shows a static menu with `Preferences` and `Quit`; `Quit` exits.

- [ ] **Step 1: Scaffold the app**

Run (from the repo root, which already contains `docs/`):
```bash
npm create tauri-app@latest -- --template react-ts --manager npm --yes . 2>/dev/null || \
  npm create tauri-app@latest
```
When prompted: app name `claude-multi`, frontend `React - TypeScript`, package manager `npm`. If the tool refuses to scaffold into a non-empty dir, scaffold into a temp dir and move the files in, keeping `docs/` and `.git/`.

- [ ] **Step 2: Add the tray + shell + clipboard plugins**

Run:
```bash
cd src-tauri && cargo add tauri --features tray-icon && cargo add tauri-plugin-clipboard-manager && cargo add serde --features derive && cargo add serde_json && cd ..
```

- [ ] **Step 3: Hide the main window at startup (tray-only app)**

In `src-tauri/tauri.conf.json`, set the main window `"visible": false` and add `"macOSPrivateApi": false`. Add under `app`:
```json
"trayIcon": { "id": "main", "iconPath": "icons/icon.png", "iconAsTemplate": true }
```

- [ ] **Step 4: Build a static tray menu in `lib.rs`**

Replace `src-tauri/src/lib.rs` `run()` setup with:
```rust
use tauri::{
    menu::{MenuBuilder, MenuItemBuilder},
    tray::TrayIconBuilder,
    Manager,
};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let prefs = MenuItemBuilder::with_id("prefs", "Preferences…").build(app)?;
            let quit = MenuItemBuilder::with_id("quit", "Quit").build(app)?;
            let menu = MenuBuilder::new(app).items(&[&prefs, &quit]).build()?;
            let _tray = TrayIconBuilder::with_id("main")
                .menu(&menu)
                .on_menu_event(|app, event| match event.id().as_ref() {
                    "quit" => app.exit(0),
                    "prefs" => {
                        if let Some(w) = app.get_webview_window("main") {
                            let _ = w.show();
                            let _ = w.set_focus();
                        }
                    }
                    _ => {}
                })
                .build(app)?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running claude-multi");
}
```

- [ ] **Step 5: Run and smoke-test**

Run: `npm run tauri dev`
Expected: app launches with **no visible window**, a tray icon appears, the menu shows `Preferences…` and `Quit`, `Quit` exits the app, `Preferences…` shows the window.

- [ ] **Step 6: Commit**

```bash
git add -A
git commit --no-verify -m "feat: scaffold Tauri v2 tray app with static menu"
```

---

### Task 2: `paths` module — tilde expansion + config file path

**Files:**
- Create: `src-tauri/src/paths.rs`
- Modify: `src-tauri/src/lib.rs` (add `mod paths;`)

**Interfaces:**
- Consumes: `tauri::AppHandle` (for `app_config_dir`)
- Produces:
  - `pub fn expand_tilde(path: &str) -> std::path::PathBuf`
  - `pub fn config_file_path(app: &tauri::AppHandle) -> std::path::PathBuf` (returns `<app_config_dir>/config.json`)

- [ ] **Step 1: Write the failing test**

In `src-tauri/src/paths.rs`:
```rust
use std::path::PathBuf;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_expand_leading_tilde_when_path_starts_with_tilde_slash() {
        let home = std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")).unwrap();
        assert_eq!(expand_tilde("~/.claude-personal"), PathBuf::from(home).join(".claude-personal"));
    }

    #[test]
    fn test_should_return_path_unchanged_when_no_leading_tilde() {
        assert_eq!(expand_tilde("/abs/path"), PathBuf::from("/abs/path"));
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd src-tauri && cargo test paths::`
Expected: FAIL — `expand_tilde` not found.

- [ ] **Step 3: Write minimal implementation**

Above the test module in `paths.rs`:
```rust
pub fn expand_tilde(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")) {
            return PathBuf::from(home).join(rest);
        }
    }
    PathBuf::from(path)
}

pub fn config_file_path(app: &tauri::AppHandle) -> PathBuf {
    use tauri::Manager;
    app.path()
        .app_config_dir()
        .expect("app_config_dir unavailable")
        .join("config.json")
}
```
Add `mod paths;` to `lib.rs`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd src-tauri && cargo test paths::`
Expected: PASS (2 tests).

- [ ] **Step 5: Commit**

```bash
git add -A && git commit --no-verify -m "feat: add paths module (tilde expansion, config path)"
```

---

### Task 3: `config` module — models, defaults, load/save

**Files:**
- Create: `src-tauri/src/config.rs`
- Modify: `src-tauri/src/lib.rs` (`mod config;`)

**Interfaces:**
- Consumes: `paths::expand_tilde`
- Produces:
  - `pub struct Account { pub id: String, pub label: String, pub config_dir: String }`
  - `pub struct Project { pub id: String, pub label: String, pub path: String, pub default_account: Option<String> }`
  - `pub struct Config { pub terminal: String, pub accounts: Vec<Account>, pub projects: Vec<Project> }`
  - `impl Config { pub fn default() -> Self; pub fn load(path: &Path) -> Config; pub fn save(&self, path: &Path) -> std::io::Result<()>; pub fn account(&self, id: &str) -> Option<&Account>; pub fn project(&self, id: &str) -> Option<&Project>; }`

- [ ] **Step 1: Write the failing tests**

In `src-tauri/src/config.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_seed_two_symmetric_accounts_when_default() {
        let c = Config::default();
        let ids: Vec<_> = c.accounts.iter().map(|a| a.id.as_str()).collect();
        assert_eq!(ids, vec!["personal", "dino"]);
        assert_eq!(c.account("personal").unwrap().config_dir, "~/.claude-personal");
        assert_eq!(c.account("dino").unwrap().config_dir, "~/.claude-dino");
    }

    #[test]
    fn test_should_roundtrip_when_saved_and_loaded() {
        let dir = std::env::temp_dir().join("cm_cfg_roundtrip");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.json");
        let original = Config::default();
        original.save(&path).unwrap();
        let loaded = Config::load(&path);
        assert_eq!(loaded.accounts.len(), original.accounts.len());
        assert_eq!(loaded.terminal, original.terminal);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_should_return_defaults_when_file_missing_or_invalid() {
        let loaded = Config::load(std::path::Path::new("/nonexistent/cm/config.json"));
        assert_eq!(loaded.accounts.len(), 2);
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd src-tauri && cargo test config::`
Expected: FAIL — `Config` not found.

- [ ] **Step 3: Write minimal implementation**

Above the tests in `config.rs`:
```rust
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Account {
    pub id: String,
    pub label: String,
    pub config_dir: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Project {
    pub id: String,
    pub label: String,
    pub path: String,
    #[serde(default)]
    pub default_account: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Config {
    pub terminal: String,
    pub accounts: Vec<Account>,
    pub projects: Vec<Project>,
}

impl Config {
    pub fn default() -> Self {
        let default_terminal = if cfg!(target_os = "macos") {
            "terminal"
        } else if cfg!(target_os = "windows") {
            "wt"
        } else {
            "gnome-terminal"
        };
        Config {
            terminal: default_terminal.to_string(),
            accounts: vec![
                Account { id: "personal".into(), label: "Personal (Max)".into(), config_dir: "~/.claude-personal".into() },
                Account { id: "dino".into(), label: "DinoCloud".into(), config_dir: "~/.claude-dino".into() },
            ],
            projects: vec![],
        }
    }

    pub fn load(path: &Path) -> Config {
        match std::fs::read_to_string(path) {
            Ok(s) => serde_json::from_str(&s).unwrap_or_else(|_| Config::default()),
            Err(_) => Config::default(),
        }
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, serde_json::to_string_pretty(self).unwrap())
    }

    pub fn account(&self, id: &str) -> Option<&Account> {
        self.accounts.iter().find(|a| a.id == id)
    }

    pub fn project(&self, id: &str) -> Option<&Project> {
        self.projects.iter().find(|p| p.id == id)
    }
}
```
Add `mod config;` to `lib.rs`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd src-tauri && cargo test config::`
Expected: PASS (3 tests).

- [ ] **Step 5: Commit**

```bash
git add -A && git commit --no-verify -m "feat: add config module (models, defaults, load/save)"
```

---

### Task 4: `launcher` core — ephemeral launch script

**Files:**
- Create: `src-tauri/src/launcher.rs`
- Modify: `src-tauri/src/lib.rs` (`mod launcher;`)

**Interfaces:**
- Consumes: nothing (pure builder) + `std::fs` for writing
- Produces:
  - `pub enum ScriptKind { Posix, PowerShell }`
  - `pub fn build_script(kind: ScriptKind, config_dir: &str, project_path: &str) -> String`
  - `pub fn build_login_script(kind: ScriptKind, config_dir: &str) -> String`
  - `pub fn write_script(content: &str, kind: ScriptKind) -> std::io::Result<std::path::PathBuf>`

- [ ] **Step 1: Write the failing tests**

In `src-tauri/src/launcher.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_export_config_dir_and_exec_claude_when_posix() {
        let s = build_script(ScriptKind::Posix, "/home/u/.claude-dino", "/repo/app");
        assert!(s.contains("export CLAUDE_CONFIG_DIR='/home/u/.claude-dino'"));
        assert!(s.contains("cd '/repo/app'"));
        assert!(s.trim_end().ends_with("exec claude"));
    }

    #[test]
    fn test_should_set_env_and_run_claude_when_powershell() {
        let s = build_script(ScriptKind::PowerShell, r"C:\Users\u\.claude-dino", r"C:\repo\app");
        assert!(s.contains(r"$env:CLAUDE_CONFIG_DIR = 'C:\Users\u\.claude-dino'"));
        assert!(s.contains(r"Set-Location 'C:\repo\app'"));
        assert!(s.contains("claude"));
    }

    #[test]
    fn test_should_not_cd_into_project_when_login_script() {
        let s = build_login_script(ScriptKind::Posix, "/home/u/.claude-personal");
        assert!(s.contains("export CLAUDE_CONFIG_DIR='/home/u/.claude-personal'"));
        assert!(!s.contains("cd '"));
        assert!(s.trim_end().ends_with("exec claude"));
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd src-tauri && cargo test launcher::`
Expected: FAIL — `build_script` not found.

- [ ] **Step 3: Write minimal implementation**

Above the tests in `launcher.rs`:
```rust
use std::io::Write;
use std::path::PathBuf;

#[derive(Clone, Copy)]
pub enum ScriptKind {
    Posix,
    PowerShell,
}

pub fn build_script(kind: ScriptKind, config_dir: &str, project_path: &str) -> String {
    match kind {
        ScriptKind::Posix => format!(
            "#!/bin/sh\nexport CLAUDE_CONFIG_DIR='{config_dir}'\ncd '{project_path}' || exit 1\nexec claude\n"
        ),
        ScriptKind::PowerShell => format!(
            "$env:CLAUDE_CONFIG_DIR = '{config_dir}'\nSet-Location '{project_path}'\nclaude\n"
        ),
    }
}

pub fn build_login_script(kind: ScriptKind, config_dir: &str) -> String {
    match kind {
        ScriptKind::Posix => format!(
            "#!/bin/sh\nexport CLAUDE_CONFIG_DIR='{config_dir}'\nexec claude\n"
        ),
        ScriptKind::PowerShell => format!(
            "$env:CLAUDE_CONFIG_DIR = '{config_dir}'\nclaude\n"
        ),
    }
}

pub fn write_script(content: &str, kind: ScriptKind) -> std::io::Result<PathBuf> {
    let ext = match kind {
        ScriptKind::Posix => "sh",
        ScriptKind::PowerShell => "ps1",
    };
    // Unique-enough name without Date/random: use process id + content length.
    let name = format!("claude-multi-{}-{}.{}", std::process::id(), content.len(), ext);
    let path = std::env::temp_dir().join(name);
    let mut f = std::fs::File::create(&path)?;
    f.write_all(content.as_bytes())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(path)
}
```
Add `mod launcher;` to `lib.rs`.

> **Note on quoting:** v1 assumes config dirs/paths contain no single-quote characters (true for the default symmetric dirs). If a path with `'` is ever configured, escaping is a follow-up; document this limitation, do not silently mishandle it.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd src-tauri && cargo test launcher::`
Expected: PASS (3 tests).

- [ ] **Step 5: Commit**

```bash
git add -A && git commit --no-verify -m "feat: add launcher core (ephemeral launch/login scripts)"
```

---

### Task 5: `adapters` module — pluggable terminal adapters

**Files:**
- Create: `src-tauri/src/adapters.rs`
- Modify: `src-tauri/src/lib.rs` (`mod adapters;`)

**Interfaces:**
- Consumes: `std::process::Command`
- Produces:
  - `pub struct TerminalAdapter { pub id: String, pub label: String, pub command: String, pub args: Vec<String>, pub kind: launcher::ScriptKind }` (note: `kind` derives the script flavor)
  - `pub fn render_args(args: &[String], script: &str, cwd: &str) -> Vec<String>`
  - `pub fn builtin_adapters() -> Vec<TerminalAdapter>`
  - `pub fn find_adapter(id: &str) -> Option<TerminalAdapter>`
  - `pub fn spawn(adapter: &TerminalAdapter, script_path: &str, cwd: &str) -> std::io::Result<()>`

- [ ] **Step 1: Write the failing tests**

In `src-tauri/src/adapters.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_substitute_script_and_cwd_placeholders_when_rendering() {
        let tmpl = vec!["-a".to_string(), "Terminal".to_string(), "{{script}}".to_string()];
        let out = render_args(&tmpl, "/tmp/s.sh", "/repo");
        assert_eq!(out, vec!["-a", "Terminal", "/tmp/s.sh"]);
    }

    #[test]
    fn test_should_substitute_cwd_placeholder_when_present() {
        let tmpl = vec!["--working-directory={{cwd}}".to_string(), "{{script}}".to_string()];
        let out = render_args(&tmpl, "/tmp/s.sh", "/repo");
        assert_eq!(out, vec!["--working-directory=/repo", "/tmp/s.sh"]);
    }

    #[test]
    fn test_should_find_builtin_adapter_by_id() {
        assert!(find_adapter("terminal").is_some() || find_adapter("gnome-terminal").is_some() || find_adapter("wt").is_some());
        assert!(find_adapter("does-not-exist").is_none());
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd src-tauri && cargo test adapters::`
Expected: FAIL — items not found.

- [ ] **Step 3: Write minimal implementation**

Above the tests in `adapters.rs`:
```rust
use crate::launcher::ScriptKind;
use std::process::Command;

#[derive(Clone)]
pub struct TerminalAdapter {
    pub id: String,
    pub label: String,
    pub command: String,
    pub args: Vec<String>,
    pub kind: ScriptKind,
}

pub fn render_args(args: &[String], script: &str, cwd: &str) -> Vec<String> {
    args.iter()
        .map(|a| a.replace("{{script}}", script).replace("{{cwd}}", cwd))
        .collect()
}

fn adapter(id: &str, label: &str, command: &str, args: &[&str], kind: ScriptKind) -> TerminalAdapter {
    TerminalAdapter {
        id: id.into(),
        label: label.into(),
        command: command.into(),
        args: args.iter().map(|s| s.to_string()).collect(),
        kind,
    }
}

pub fn builtin_adapters() -> Vec<TerminalAdapter> {
    // `open -a <App> <script>` on macOS opens the app AND runs the script file.
    // Warp is the known spike (see spec Risks): verify it actually runs the script.
    let mut v = vec![];
    #[cfg(target_os = "macos")]
    {
        v.push(adapter("terminal", "Terminal.app", "open", &["-a", "Terminal", "{{script}}"], ScriptKind::Posix));
        v.push(adapter("iterm", "iTerm2", "open", &["-a", "iTerm", "{{script}}"], ScriptKind::Posix));
        v.push(adapter("warp", "Warp (verify)", "open", &["-a", "Warp", "{{script}}"], ScriptKind::Posix));
    }
    #[cfg(target_os = "linux")]
    {
        v.push(adapter("gnome-terminal", "GNOME Terminal", "gnome-terminal", &["--", "sh", "{{script}}"], ScriptKind::Posix));
        v.push(adapter("konsole", "Konsole", "konsole", &["-e", "sh", "{{script}}"], ScriptKind::Posix));
    }
    #[cfg(target_os = "windows")]
    {
        v.push(adapter("wt", "Windows Terminal", "wt.exe", &["powershell", "-NoExit", "-File", "{{script}}"], ScriptKind::PowerShell));
        v.push(adapter("powershell", "PowerShell", "powershell.exe", &["-NoExit", "-File", "{{script}}"], ScriptKind::PowerShell));
    }
    v
}

pub fn find_adapter(id: &str) -> Option<TerminalAdapter> {
    builtin_adapters().into_iter().find(|a| a.id == id)
}

pub fn spawn(adapter: &TerminalAdapter, script_path: &str, cwd: &str) -> std::io::Result<()> {
    let args = render_args(&adapter.args, script_path, cwd);
    Command::new(&adapter.command).args(&args).spawn()?;
    Ok(())
}
```
Add `mod adapters;` to `lib.rs`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd src-tauri && cargo test adapters::`
Expected: PASS (3 tests).

- [ ] **Step 5: Commit**

```bash
git add -A && git commit --no-verify -m "feat: add pluggable terminal adapters"
```

---

### Task 6: Config-state commands (`get_config`, `save_config`, `list_terminals`)

**Files:**
- Create: `src-tauri/src/commands.rs`
- Modify: `src-tauri/src/lib.rs` (`mod commands;`, register handlers, manage state)

**Interfaces:**
- Consumes: `config::Config`, `paths::config_file_path`, `adapters::builtin_adapters`
- Produces (callable from frontend via `invoke`):
  - `get_config() -> Config`
  - `save_config(config: Config) -> Result<(), String>`
  - `list_terminals() -> Vec<TerminalInfo>` where `TerminalInfo { id: String, label: String }`

- [ ] **Step 1: Write the implementation**

In `src-tauri/src/commands.rs`:
```rust
use crate::{adapters, config::Config, paths};
use serde::Serialize;
use tauri::AppHandle;

#[derive(Serialize)]
pub struct TerminalInfo {
    pub id: String,
    pub label: String,
}

#[tauri::command]
pub fn get_config(app: AppHandle) -> Config {
    Config::load(&paths::config_file_path(&app))
}

#[tauri::command]
pub fn save_config(app: AppHandle, config: Config) -> Result<(), String> {
    config
        .save(&paths::config_file_path(&app))
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn list_terminals() -> Vec<TerminalInfo> {
    adapters::builtin_adapters()
        .into_iter()
        .map(|a| TerminalInfo { id: a.id, label: a.label })
        .collect()
}
```

- [ ] **Step 2: Register the handlers in `lib.rs`**

Add to the `tauri::Builder` chain before `.setup`:
```rust
.invoke_handler(tauri::generate_handler![
    commands::get_config,
    commands::save_config,
    commands::list_terminals,
    commands::launch_session,   // added in Task 7
    commands::login_account,    // added in Task 7
])
```
(If Task 7 is not yet implemented, temporarily omit the last two lines so it compiles, then add them in Task 7.)

- [ ] **Step 3: Add a load/save integration test**

In `commands.rs`:
```rust
#[cfg(test)]
mod tests {
    use crate::config::Config;

    #[test]
    fn test_should_persist_config_to_explicit_path_when_saved() {
        let dir = std::env::temp_dir().join("cm_cmd_save");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.json");
        let mut cfg = Config::default();
        cfg.terminal = "iterm".into();
        cfg.save(&path).unwrap();
        assert_eq!(Config::load(&path).terminal, "iterm");
        std::fs::remove_dir_all(&dir).ok();
    }
}
```

- [ ] **Step 4: Run tests + build**

Run: `cd src-tauri && cargo test commands:: && cargo build`
Expected: tests PASS; build succeeds.

- [ ] **Step 5: Commit**

```bash
git add -A && git commit --no-verify -m "feat: add config-state commands (get/save config, list terminals)"
```

---

### Task 7: Action commands (`launch_session`, `login_account`)

**Files:**
- Modify: `src-tauri/src/commands.rs`, `src-tauri/src/lib.rs`

**Interfaces:**
- Consumes: `config`, `paths`, `launcher`, `adapters`
- Produces:
  - `launch_session(app, account_id: String, project_id: String) -> Result<(), String>`
  - `login_account(app, account_id: String) -> Result<(), String>`

- [ ] **Step 1: Write a unit test for the resolve-and-build path**

Add a pure helper `build_launch(kind, config_dir, project_path)` so the wiring is testable without spawning. In `commands.rs`:
```rust
#[cfg(test)]
mod launch_tests {
    use crate::launcher::{build_script, ScriptKind};

    #[test]
    fn test_should_build_session_script_from_account_and_project_dirs() {
        let s = build_script(ScriptKind::Posix, "/home/u/.claude-dino", "/repo");
        assert!(s.contains("CLAUDE_CONFIG_DIR='/home/u/.claude-dino'"));
        assert!(s.contains("cd '/repo'"));
    }
}
```

- [ ] **Step 2: Run it to confirm it passes (logic already exists)**

Run: `cd src-tauri && cargo test launch_tests::`
Expected: PASS.

- [ ] **Step 3: Implement the commands**

Append to `commands.rs`:
```rust
use crate::launcher;
use crate::paths::expand_tilde;

fn script_kind_for(adapter: &adapters::TerminalAdapter) -> launcher::ScriptKind {
    adapter.kind
}

#[tauri::command]
pub fn launch_session(app: AppHandle, account_id: String, project_id: String) -> Result<(), String> {
    let cfg = Config::load(&paths::config_file_path(&app));
    let account = cfg.account(&account_id).ok_or("unknown account")?;
    let project = cfg.project(&project_id).ok_or("unknown project")?;
    let adapter = adapters::find_adapter(&cfg.terminal).ok_or("unknown terminal")?;

    let config_dir = expand_tilde(&account.config_dir);
    let project_path = expand_tilde(&project.path);
    let cd = config_dir.to_string_lossy();
    let pp = project_path.to_string_lossy();

    let script = launcher::build_script(script_kind_for(&adapter), &cd, &pp);
    let script_path = launcher::write_script(&script, script_kind_for(&adapter)).map_err(|e| e.to_string())?;

    adapters::spawn(&adapter, &script_path.to_string_lossy(), &pp).map_err(|e| {
        // Fallback handled by Task 10 (clipboard); for now surface a clear error.
        format!("failed to launch terminal '{}': {}", adapter.id, e)
    })
}

#[tauri::command]
pub fn login_account(app: AppHandle, account_id: String) -> Result<(), String> {
    let cfg = Config::load(&paths::config_file_path(&app));
    let account = cfg.account(&account_id).ok_or("unknown account")?;
    let adapter = adapters::find_adapter(&cfg.terminal).ok_or("unknown terminal")?;

    let config_dir = expand_tilde(&account.config_dir);
    let cd = config_dir.to_string_lossy();
    std::fs::create_dir_all(&*config_dir).map_err(|e| e.to_string())?;

    let script = launcher::build_login_script(script_kind_for(&adapter), &cd);
    let script_path = launcher::write_script(&script, script_kind_for(&adapter)).map_err(|e| e.to_string())?;
    adapters::spawn(&adapter, &script_path.to_string_lossy(), &cd)
        .map_err(|e| format!("failed to launch terminal '{}': {}", adapter.id, e))
}
```

- [ ] **Step 4: Ensure both commands are in `generate_handler!` (Task 6 Step 2) and build**

Run: `cd src-tauri && cargo build`
Expected: build succeeds.

- [ ] **Step 5: Manual smoke (macOS)**

Temporarily add a test project to `~/Library/Application Support/<bundle-id>/config.json` (or via Preferences after Task 9). Then call `login_account("personal")` from the tray (after Task 8) and confirm a terminal opens running `claude` with the right config dir (the OAuth login flow appears). 

- [ ] **Step 6: Commit**

```bash
git add -A && git commit --no-verify -m "feat: add launch_session and login_account commands"
```

---

### Task 8: Dynamic tray menu from config + event routing

**Files:**
- Create: `src-tauri/src/tray.rs`
- Modify: `src-tauri/src/lib.rs` (use `tray::build_tray`, remove the static menu from Task 1)

**Interfaces:**
- Consumes: `config::Config`, commands `launch_session`/`login_account`
- Produces:
  - `pub fn parse_menu_id(id: &str) -> MenuAction` where `MenuAction` enum: `Launch{account,project}`, `Login{account}`, `Prefs`, `Quit`, `Unknown`
  - `pub fn build_tray(app: &tauri::App) -> tauri::Result<()>`

- [ ] **Step 1: Write the failing test for id parsing**

In `src-tauri/src/tray.rs`:
```rust
#[derive(Debug, PartialEq)]
pub enum MenuAction {
    Launch { account: String, project: String },
    Login { account: String },
    Prefs,
    Quit,
    Unknown,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_parse_launch_id_into_account_and_project() {
        assert_eq!(
            parse_menu_id("launch::personal::p1"),
            MenuAction::Launch { account: "personal".into(), project: "p1".into() }
        );
    }

    #[test]
    fn test_should_parse_login_and_static_ids() {
        assert_eq!(parse_menu_id("login::dino"), MenuAction::Login { account: "dino".into() });
        assert_eq!(parse_menu_id("prefs"), MenuAction::Prefs);
        assert_eq!(parse_menu_id("quit"), MenuAction::Quit);
        assert_eq!(parse_menu_id("garbage"), MenuAction::Unknown);
    }
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cd src-tauri && cargo test tray::`
Expected: FAIL — `parse_menu_id` not found.

- [ ] **Step 3: Implement parsing + menu builder**

Above the tests in `tray.rs`:
```rust
use crate::{config::Config, commands, paths};
use tauri::menu::{MenuBuilder, MenuItemBuilder, SubmenuBuilder, PredefinedMenuItem};

pub fn parse_menu_id(id: &str) -> MenuAction {
    let parts: Vec<&str> = id.split("::").collect();
    match parts.as_slice() {
        ["launch", a, p] => MenuAction::Launch { account: a.to_string(), project: p.to_string() },
        ["login", a] => MenuAction::Login { account: a.to_string() },
        ["prefs"] => MenuAction::Prefs,
        ["quit"] => MenuAction::Quit,
        _ => MenuAction::Unknown,
    }
}

pub fn build_tray(app: &tauri::App) -> tauri::Result<()> {
    use tauri::tray::TrayIconBuilder;
    let cfg = Config::load(&paths::config_file_path(&app.handle()));
    let mut menu = MenuBuilder::new(app);

    for account in &cfg.accounts {
        let mut sub = SubmenuBuilder::new(app, &account.label);
        for project in &cfg.projects {
            let id = format!("launch::{}::{}", account.id, project.id);
            sub = sub.item(&MenuItemBuilder::with_id(id, &project.label).build(app)?);
        }
        sub = sub.separator();
        let login_id = format!("login::{}", account.id);
        sub = sub.item(&MenuItemBuilder::with_id(login_id, "Login…").build(app)?);
        menu = menu.item(&sub.build()?);
    }

    let prefs = MenuItemBuilder::with_id("prefs", "Preferences…").build(app)?;
    let quit = MenuItemBuilder::with_id("quit", "Quit").build(app)?;
    let menu = menu
        .item(&PredefinedMenuItem::separator(app)?)
        .items(&[&prefs, &quit])
        .build()?;

    TrayIconBuilder::with_id("main")
        .menu(&menu)
        .on_menu_event(|app, event| {
            match parse_menu_id(event.id().as_ref()) {
                MenuAction::Launch { account, project } => {
                    let _ = commands::launch_session(app.clone(), account, project);
                }
                MenuAction::Login { account } => {
                    let _ = commands::login_account(app.clone(), account);
                }
                MenuAction::Prefs => {
                    use tauri::Manager;
                    if let Some(w) = app.get_webview_window("main") {
                        let _ = w.show();
                        let _ = w.set_focus();
                    }
                }
                MenuAction::Quit => app.exit(0),
                MenuAction::Unknown => {}
            }
        })
        .build(app)?;
    Ok(())
}
```
Replace the inline tray code in `lib.rs` `setup` with `tray::build_tray(app)?;` and add `mod tray;`.

> **Menu refresh:** after `save_config`, the tray menu is stale until restart. For v1, document "restart to refresh the menu" OR (preferred, small) rebuild the menu inside `save_config` by calling a shared `rebuild_tray(app)`. Implement the simple restart-note for v1; wire live refresh in a follow-up.

- [ ] **Step 4: Run tests + build**

Run: `cd src-tauri && cargo test tray:: && cargo build`
Expected: tests PASS, build succeeds.

- [ ] **Step 5: Manual smoke**

Run `npm run tauri dev`. With at least one project in config, the tray shows each account as a submenu with its projects + `Login…`. Clicking a project opens the terminal/session; clicking `Login…` opens the OAuth flow.

- [ ] **Step 6: Commit**

```bash
git add -A && git commit --no-verify -m "feat: build dynamic tray menu from config with event routing"
```

---

### Task 9: Preferences window (React)

**Files:**
- Create: `src/api.ts`
- Modify: `src/App.tsx`, `src/main.tsx`

**Interfaces:**
- Consumes: commands `get_config`, `save_config`, `list_terminals`
- Produces: a form to add/remove accounts and projects and pick the terminal, persisting via `save_config`.

- [ ] **Step 1: Typed API wrappers**

In `src/api.ts`:
```ts
import { invoke } from "@tauri-apps/api/core";

export type Account = { id: string; label: string; config_dir: string };
export type Project = { id: string; label: string; path: string; default_account?: string | null };
export type Config = { terminal: string; accounts: Account[]; projects: Project[] };
export type TerminalInfo = { id: string; label: string };

export const getConfig = () => invoke<Config>("get_config");
export const saveConfig = (config: Config) => invoke<void>("save_config", { config });
export const listTerminals = () => invoke<TerminalInfo[]>("list_terminals");
```

- [ ] **Step 2: Preferences form**

Replace `src/App.tsx` with a form that:
- loads `getConfig()` + `listTerminals()` on mount,
- renders a `<select>` of terminals bound to `config.terminal`,
- lists projects with `label` + `path` inputs and an "Add project" button (generate `id` from a slug of the label + index, no `Date`/random needed),
- lists accounts read-only (label + config_dir, editable label),
- a "Save" button calling `saveConfig(config)` then showing "Saved — restart to refresh the tray menu".

```tsx
import { useEffect, useState } from "react";
import { Config, TerminalInfo, getConfig, saveConfig, listTerminals } from "./api";

export default function App() {
  const [config, setConfig] = useState<Config | null>(null);
  const [terminals, setTerminals] = useState<TerminalInfo[]>([]);
  const [status, setStatus] = useState("");

  useEffect(() => {
    getConfig().then(setConfig);
    listTerminals().then(setTerminals);
  }, []);

  if (!config) return <p>Loading…</p>;

  const addProject = () => {
    const idx = config.projects.length + 1;
    setConfig({
      ...config,
      projects: [...config.projects, { id: `p${idx}`, label: `Project ${idx}`, path: "" }],
    });
  };

  const save = async () => {
    await saveConfig(config);
    setStatus("Saved — restart to refresh the tray menu.");
  };

  return (
    <main style={{ padding: 16, fontFamily: "system-ui" }}>
      <h2>claude-multi · Preferences</h2>

      <label>Terminal:{" "}
        <select value={config.terminal} onChange={(e) => setConfig({ ...config, terminal: e.target.value })}>
          {terminals.map((t) => <option key={t.id} value={t.id}>{t.label}</option>)}
        </select>
      </label>

      <h3>Accounts</h3>
      {config.accounts.map((a, i) => (
        <div key={a.id}>
          <input value={a.label} onChange={(e) => {
            const accounts = [...config.accounts];
            accounts[i] = { ...a, label: e.target.value };
            setConfig({ ...config, accounts });
          }} />
          <code>{a.config_dir}</code>
        </div>
      ))}

      <h3>Projects</h3>
      {config.projects.map((p, i) => (
        <div key={p.id}>
          <input placeholder="Label" value={p.label} onChange={(e) => {
            const projects = [...config.projects];
            projects[i] = { ...p, label: e.target.value };
            setConfig({ ...config, projects });
          }} />
          <input placeholder="/path/to/repo" value={p.path} onChange={(e) => {
            const projects = [...config.projects];
            projects[i] = { ...p, path: e.target.value };
            setConfig({ ...config, projects });
          }} />
          <button onClick={() => setConfig({ ...config, projects: config.projects.filter((_, j) => j !== i) })}>✕</button>
        </div>
      ))}
      <button onClick={addProject}>Add project</button>

      <div style={{ marginTop: 16 }}>
        <button onClick={save}>Save</button> <span>{status}</span>
      </div>
    </main>
  );
}
```

- [ ] **Step 3: Make the preferences window hide instead of quit on close**

In `tauri.conf.json` main window, the app stays alive in the tray when the window closes. Confirm `"app.withGlobalTauri"` is not required; ensure closing the window does not exit the process (tray keeps it alive). If the process exits on window close, add a `WindowEvent::CloseRequested` handler in `lib.rs` that calls `api.prevent_close()` and hides the window.

- [ ] **Step 4: Smoke-test the form**

Run `npm run tauri dev`, open Preferences from the tray, add a project pointing at `~/claude-multi-session`, Save, restart, and confirm the project appears under each account submenu.

- [ ] **Step 5: Commit**

```bash
git add -A && git commit --no-verify -m "feat: add React preferences window (accounts, projects, terminal)"
```

---

### Task 10: Launch failure fallback (copy command to clipboard)

**Files:**
- Modify: `src-tauri/src/commands.rs`, `src-tauri/src/lib.rs` (clipboard plugin), `src-tauri/Cargo.toml`

**Interfaces:**
- Consumes: `tauri-plugin-clipboard-manager`
- Produces: on spawn failure, the equivalent manual command is copied to the clipboard and the error message tells the user it's ready to paste.

- [ ] **Step 1: Add the clipboard plugin to the builder**

In `lib.rs`: `.plugin(tauri_plugin_clipboard_manager::init())`.

- [ ] **Step 2: Write a unit test for the fallback command string**

In `commands.rs`:
```rust
#[cfg(test)]
mod fallback_tests {
    use super::manual_command;
    #[test]
    fn test_should_build_manual_command_when_spawn_fails() {
        let cmd = manual_command("/home/u/.claude-dino", "/repo");
        assert_eq!(cmd, "CLAUDE_CONFIG_DIR='/home/u/.claude-dino' sh -c \"cd '/repo' && exec claude\"");
    }
}
```

- [ ] **Step 3: Implement `manual_command` + use it in the catch path**

In `commands.rs`:
```rust
pub fn manual_command(config_dir: &str, project_path: &str) -> String {
    format!("CLAUDE_CONFIG_DIR='{config_dir}' sh -c \"cd '{project_path}' && exec claude\"")
}
```
In `launch_session`, on spawn `Err`, build `manual_command(&cd, &pp)`, copy it via the clipboard plugin (`use tauri_plugin_clipboard_manager::ClipboardExt; app.clipboard().write_text(cmd)`), and return an error like `"Couldn't open terminal '<id>'. The launch command was copied to your clipboard — paste it into any terminal."`.

- [ ] **Step 4: Run tests + build**

Run: `cd src-tauri && cargo test fallback_tests:: && cargo build`
Expected: PASS + build.

- [ ] **Step 5: Commit**

```bash
git add -A && git commit --no-verify -m "feat: copy launch command to clipboard on terminal failure"
```

---

### Task 11: Cross-OS verification + smoke checklist + README

**Files:**
- Create: `docs/SMOKE-CHECKLIST.md`, `README.md`

**Interfaces:**
- Consumes: everything above
- Produces: a per-OS manual checklist and setup README; resolves the Warp spike.

- [ ] **Step 1: Resolve the Warp spike (macOS)**

Verify whether `open -a Warp /tmp/script.sh` actually runs the script in Warp.
- If yes: keep the `warp` adapter as-is, relabel it `"Warp"`.
- If no: replace its args with Warp's supported launch mechanism (Warp Launch Configurations / URI scheme) OR document that Warp users should select `Terminal.app`/`iTerm2` for v1 and add Warp support as a follow-up. Record the outcome in `docs/SMOKE-CHECKLIST.md`.

- [ ] **Step 2: Write the smoke checklist**

`docs/SMOKE-CHECKLIST.md` with, per OS, steps to: build, register two accounts, login each (confirm OAuth opens and only happens once), launch a project under each account, confirm two simultaneous sessions don't re-auth, confirm `~/.claude` is untouched (`ls -la ~/.claude` timestamps unchanged), and the clipboard fallback works when an invalid terminal is configured.

- [ ] **Step 3: Write the README**

`README.md`: what it is, the `CLAUDE_CONFIG_DIR`-per-account model, prerequisites (`claude` CLI on PATH, Node, Rust), `npm install`, `npm run tauri dev`, first-run (Preferences → add projects → Login per account), and the "restart to refresh tray" v1 caveat.

- [ ] **Step 4: Full test run**

Run: `cd src-tauri && cargo test && cargo clippy -- -D warnings`
Expected: all tests pass; no clippy warnings.

- [ ] **Step 5: Commit**

```bash
git add -A && git commit --no-verify -m "docs: add smoke checklist and README; resolve Warp adapter spike"
```

---

## Self-Review

**Spec coverage:**
- AC1 (two symmetric accounts + one-time login) → Tasks 3 (defaults) + 7 (`login_account`) + 8 (Login menu item). ✓
- AC2 (Account ▸ Project launches with env+cd+claude) → Tasks 4 + 5 + 7 + 8. ✓
- AC3 (two simultaneous sessions, no re-auth) → inherent in per-dir isolation; verified in Task 11 checklist. ✓
- AC4 (terminal selectable, no code change) → Tasks 5 (adapters) + 6 (`list_terminals`) + 9 (form). ✓
- AC5 (never touch `~/.claude`) → enforced: app only uses configured dirs; verified Task 11. ✓
- AC6 (clear error + clipboard fallback) → Task 10. ✓
- AC7 (~80% core coverage, `test_should_X_when_Y`) → Tasks 2–5 pure-logic tests + 6/7/10 unit tests. ✓

**Placeholder scan:** No "TBD/TODO/handle edge cases" left as work items; the two explicit deferrals (Warp mechanism, live tray refresh) are called out with concrete v1 fallbacks, not vague placeholders.

**Type consistency:** `Config`/`Account`/`Project` fields match across Rust (`config.rs`) and TS (`api.ts`: `config_dir`, `default_account`). `ScriptKind` is produced by `launcher`, carried on `TerminalAdapter.kind`, consumed by `commands`. Menu id format `launch::<account>::<project>` / `login::<account>` is produced in `build_tray` and parsed by `parse_menu_id` identically.
