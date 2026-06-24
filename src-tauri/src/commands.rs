use crate::{adapters, config::Config, launcher, paths};
use crate::paths::expand_tilde;
use serde::Serialize;
use tauri::AppHandle;
use tauri_plugin_clipboard_manager::ClipboardExt;

pub fn manual_command(config_dir: &str, project_path: &str) -> String {
    format!("CLAUDE_CONFIG_DIR='{config_dir}' sh -c \"cd '{project_path}' && exec claude\"")
}

pub fn manual_login_command(config_dir: &str) -> String {
    format!("CLAUDE_CONFIG_DIR='{config_dir}' sh -c \"exec claude\"")
}

pub fn manual_logout_command(config_dir: &str) -> String {
    format!("CLAUDE_CONFIG_DIR='{config_dir}' sh -c \"claude auth logout\"")
}

pub fn manual_relogin_command(config_dir: &str) -> String {
    format!("CLAUDE_CONFIG_DIR='{config_dir}' sh -c \"claude auth logout; claude auth login\"")
}

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
        .map_err(|e| e.to_string())?;
    // Rebuild the tray menu so changes apply immediately, no restart needed.
    let _ = crate::tray::refresh_tray(&app);
    Ok(())
}

#[tauri::command]
pub fn list_terminals() -> Vec<TerminalInfo> {
    adapters::builtin_adapters()
        .into_iter()
        .map(|a| TerminalInfo { id: a.id, label: a.label })
        .collect()
}

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

    adapters::spawn(&adapter, &script_path.to_string_lossy(), &pp).map_err(|_e| {
        let cmd = manual_command(&cd, &pp);
        let _ = app.clipboard().write_text(cmd);
        format!(
            "Couldn't open terminal '{}'. The launch command was copied to your clipboard — paste it into any terminal.",
            adapter.id
        )
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
    adapters::spawn(&adapter, &script_path.to_string_lossy(), &cd).map_err(|_e| {
        let cmd = manual_login_command(&cd);
        let _ = app.clipboard().write_text(cmd);
        format!(
            "Couldn't open terminal '{}' for login. The login command was copied to your clipboard — paste it into any terminal.",
            adapter.id
        )
    })
}

#[tauri::command]
pub fn open_session(app: AppHandle, account_id: String) -> Result<(), String> {
    // A session under an account but outside any project: same as launching
    // `claude` in the account's config dir with no `cd` into a project.
    let cfg = Config::load(&paths::config_file_path(&app));
    let account = cfg.account(&account_id).ok_or("unknown account")?;
    let adapter = adapters::find_adapter(&cfg.terminal).ok_or("unknown terminal")?;

    let config_dir = expand_tilde(&account.config_dir);
    let cd = config_dir.to_string_lossy();
    std::fs::create_dir_all(&*config_dir).map_err(|e| e.to_string())?;

    let script = launcher::build_login_script(script_kind_for(&adapter), &cd);
    let script_path = launcher::write_script(&script, script_kind_for(&adapter)).map_err(|e| e.to_string())?;
    adapters::spawn(&adapter, &script_path.to_string_lossy(), &cd).map_err(|_e| {
        let cmd = manual_login_command(&cd);
        let _ = app.clipboard().write_text(cmd);
        format!(
            "Couldn't open terminal '{}' for the session. The command was copied to your clipboard — paste it into any terminal.",
            adapter.id
        )
    })
}

#[tauri::command]
pub fn logout_account(app: AppHandle, account_id: String) -> Result<(), String> {
    let cfg = Config::load(&paths::config_file_path(&app));
    let account = cfg.account(&account_id).ok_or("unknown account")?;
    let adapter = adapters::find_adapter(&cfg.terminal).ok_or("unknown terminal")?;

    let config_dir = expand_tilde(&account.config_dir);
    let cd = config_dir.to_string_lossy();

    let script = launcher::build_logout_script(script_kind_for(&adapter), &cd);
    let script_path = launcher::write_script(&script, script_kind_for(&adapter)).map_err(|e| e.to_string())?;
    adapters::spawn(&adapter, &script_path.to_string_lossy(), &cd).map_err(|_e| {
        let cmd = manual_logout_command(&cd);
        let _ = app.clipboard().write_text(cmd);
        format!(
            "Couldn't open terminal '{}' for logout. The logout command was copied to your clipboard — paste it into any terminal.",
            adapter.id
        )
    })
}

#[tauri::command]
pub fn relogin_account(app: AppHandle, account_id: String) -> Result<(), String> {
    let cfg = Config::load(&paths::config_file_path(&app));
    let account = cfg.account(&account_id).ok_or("unknown account")?;
    let adapter = adapters::find_adapter(&cfg.terminal).ok_or("unknown terminal")?;

    let config_dir = expand_tilde(&account.config_dir);
    let cd = config_dir.to_string_lossy();
    std::fs::create_dir_all(&*config_dir).map_err(|e| e.to_string())?;

    let script = launcher::build_relogin_script(script_kind_for(&adapter), &cd);
    let script_path = launcher::write_script(&script, script_kind_for(&adapter)).map_err(|e| e.to_string())?;
    adapters::spawn(&adapter, &script_path.to_string_lossy(), &cd).map_err(|_e| {
        let cmd = manual_relogin_command(&cd);
        let _ = app.clipboard().write_text(cmd);
        format!(
            "Couldn't open terminal '{}' for re-login. The re-login command was copied to your clipboard — paste it into any terminal.",
            adapter.id
        )
    })
}

#[cfg(test)]
mod fallback_tests {
    use super::{manual_command, manual_login_command, manual_logout_command, manual_relogin_command};
    #[test]
    fn test_should_build_manual_command_when_spawn_fails() {
        let cmd = manual_command("/home/u/.claude-dino", "/repo");
        assert_eq!(cmd, "CLAUDE_CONFIG_DIR='/home/u/.claude-dino' sh -c \"cd '/repo' && exec claude\"");
    }
    #[test]
    fn test_should_build_manual_login_command_when_spawn_fails() {
        let cmd = manual_login_command("/home/u/.claude-dino");
        assert_eq!(cmd, "CLAUDE_CONFIG_DIR='/home/u/.claude-dino' sh -c \"exec claude\"");
    }
    #[test]
    fn test_should_build_manual_logout_command_when_spawn_fails() {
        let cmd = manual_logout_command("/home/u/.claude-dino");
        assert_eq!(cmd, "CLAUDE_CONFIG_DIR='/home/u/.claude-dino' sh -c \"claude auth logout\"");
    }
    #[test]
    fn test_should_build_manual_relogin_command_when_spawn_fails() {
        let cmd = manual_relogin_command("/home/u/.claude-dino");
        assert_eq!(cmd, "CLAUDE_CONFIG_DIR='/home/u/.claude-dino' sh -c \"claude auth logout; claude auth login\"");
    }
}

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
