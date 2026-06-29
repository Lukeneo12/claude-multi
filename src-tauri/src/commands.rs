use crate::config::{Config, InheritDecision};
use crate::paths::expand_tilde;
use crate::{adapters, inherit, launcher, paths};
use serde::Serialize;
use tauri::AppHandle;
use tauri_plugin_clipboard_manager::ClipboardExt;

/// Ask the user, once per subdir, whether to merge the shared resources into an
/// account that already has its own. Returns the chosen decision.
fn prompt_inherit_decision(app: &AppHandle, account_id: &str, subdir: &str) -> InheritDecision {
    use tauri_plugin_dialog::{DialogExt, MessageDialogButtons};
    let merge = app
        .dialog()
        .message(format!(
            "Account '{account_id}' has its own '{subdir}'. Also inherit the shared \
             '{subdir}' from ~/.claude?\n\n\
             Merge = add the shared ones (your own files are kept).\n\
             Skip = keep this account's '{subdir}' isolated."
        ))
        .title("Inherit shared resources?")
        .buttons(MessageDialogButtons::OkCancelCustom(
            "Merge".into(),
            "Skip".into(),
        ))
        .blocking_show();
    if merge {
        InheritDecision::Merge
    } else {
        InheritDecision::Skip
    }
}

/// Ensure one account's config dir inherits `~/.claude` resources before launch.
/// Prompts once per unresolved conflict, persists the decision, then re-applies.
/// No-op (Ok) when `~/.claude` doesn't exist.
fn ensure_account_inherits(app: &AppHandle, account_id: &str) -> Result<(), String> {
    let source = expand_tilde("~/.claude");
    if !source.is_dir() {
        return Ok(()); // nothing to inherit from
    }

    let cfg_path = paths::config_file_path(app);
    let mut cfg = Config::load(&cfg_path);

    let config_dir = expand_tilde(&cfg.account(account_id).ok_or("unknown account")?.config_dir);
    let decisions = cfg
        .account(account_id)
        .map(|a| a.inherit_overrides.clone())
        .unwrap_or_default();

    let outcome =
        inherit::ensure_inherited(&source, &config_dir, &decisions).map_err(|e| e.to_string())?;
    if outcome.needs_prompt.is_empty() {
        return Ok(());
    }

    // Prompt once per conflicted subdir, then persist and re-apply.
    let mut new_decisions = decisions;
    for sub in &outcome.needs_prompt {
        new_decisions.insert(sub.clone(), prompt_inherit_decision(app, account_id, sub));
    }
    if let Some(account) = cfg.accounts.iter_mut().find(|a| a.id == account_id) {
        account.inherit_overrides = new_decisions.clone();
    }
    cfg.save(&cfg_path).map_err(|e| e.to_string())?;

    inherit::ensure_inherited(&source, &config_dir, &new_decisions).map_err(|e| e.to_string())?;
    Ok(())
}

/// Builds the paste-able fallback command: `CLAUDE_CONFIG_DIR='…' sh -c "<inner>"`.
fn manual_sh(config_dir: &str, inner: &str) -> String {
    format!("CLAUDE_CONFIG_DIR='{config_dir}' sh -c \"{inner}\"")
}

pub fn manual_command(config_dir: &str, project_path: &str) -> String {
    manual_sh(config_dir, &format!("cd '{project_path}' && exec claude"))
}

pub fn manual_login_command(config_dir: &str) -> String {
    manual_sh(config_dir, "exec claude")
}

pub fn manual_logout_command(config_dir: &str) -> String {
    manual_sh(config_dir, "claude auth logout")
}

pub fn manual_relogin_command(config_dir: &str) -> String {
    manual_sh(config_dir, "claude auth logout; claude auth login")
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
        .map(|a| TerminalInfo {
            id: a.id,
            label: a.label,
        })
        .collect()
}

#[tauri::command]
pub fn launch_session(
    app: AppHandle,
    account_id: String,
    project_id: String,
) -> Result<(), String> {
    let cfg = Config::load(&paths::config_file_path(&app));
    let account = cfg.account(&account_id).ok_or("unknown account")?;
    let project = cfg.project(&project_id).ok_or("unknown project")?;
    let adapter = adapters::find_adapter(&cfg.terminal).ok_or("unknown terminal")?;

    ensure_account_inherits(&app, &account_id)?;

    let config_dir = expand_tilde(&account.config_dir);
    let project_path = expand_tilde(&project.path);
    let cd = config_dir.to_string_lossy();
    let pp = project_path.to_string_lossy();

    let script = launcher::build_script(adapter.kind, &cd, &pp);
    let script_path = launcher::write_script(&script, adapter.kind).map_err(|e| e.to_string())?;

    adapters::spawn(&adapter, &script_path.to_string_lossy(), &pp).map_err(|_e| {
        let _ = app.clipboard().write_text(manual_command(&cd, &pp));
        format!(
            "Couldn't open terminal '{}'. The launch command was copied to your clipboard — paste it into any terminal.",
            adapter.id
        )
    })
}

/// A project-less action performed under one account's `CLAUDE_CONFIG_DIR`.
enum AccountAction {
    Login,
    Session,
    Logout,
    Relogin,
}

/// Shared flow for the project-less account actions: resolve the account and
/// terminal, build the right script, write it, and spawn — copying a paste-able
/// command to the clipboard if the terminal can't be opened.
fn run_account_action(
    app: &AppHandle,
    account_id: &str,
    action: AccountAction,
) -> Result<(), String> {
    let cfg = Config::load(&paths::config_file_path(app));
    let account = cfg.account(account_id).ok_or("unknown account")?;
    let adapter = adapters::find_adapter(&cfg.terminal).ok_or("unknown terminal")?;

    let config_dir = expand_tilde(&account.config_dir);
    let cd = config_dir.to_string_lossy();

    // Logout needs no dir creation (the account is logged in already); the other
    // actions ensure the per-account dir exists for a first-run login.
    if !matches!(action, AccountAction::Logout) {
        std::fs::create_dir_all(&*config_dir).map_err(|e| e.to_string())?;
    }

    if matches!(action, AccountAction::Login | AccountAction::Session) {
        ensure_account_inherits(app, account_id)?;
    }

    let (script, fallback, what) = match action {
        AccountAction::Login => (
            launcher::build_login_script(adapter.kind, &cd),
            manual_login_command(&cd),
            "login",
        ),
        AccountAction::Session => (
            launcher::build_login_script(adapter.kind, &cd),
            manual_login_command(&cd),
            "the session",
        ),
        AccountAction::Logout => (
            launcher::build_logout_script(adapter.kind, &cd),
            manual_logout_command(&cd),
            "logout",
        ),
        AccountAction::Relogin => (
            launcher::build_relogin_script(adapter.kind, &cd),
            manual_relogin_command(&cd),
            "re-login",
        ),
    };

    let script_path = launcher::write_script(&script, adapter.kind).map_err(|e| e.to_string())?;
    adapters::spawn(&adapter, &script_path.to_string_lossy(), &cd).map_err(|_e| {
        let _ = app.clipboard().write_text(fallback);
        format!(
            "Couldn't open terminal '{}' for {what}. The command was copied to your clipboard — paste it into any terminal.",
            adapter.id
        )
    })
}

#[tauri::command]
pub fn login_account(app: AppHandle, account_id: String) -> Result<(), String> {
    run_account_action(&app, &account_id, AccountAction::Login)
}

#[tauri::command]
pub fn open_session(app: AppHandle, account_id: String) -> Result<(), String> {
    run_account_action(&app, &account_id, AccountAction::Session)
}

#[tauri::command]
pub fn logout_account(app: AppHandle, account_id: String) -> Result<(), String> {
    run_account_action(&app, &account_id, AccountAction::Logout)
}

#[tauri::command]
pub fn relogin_account(app: AppHandle, account_id: String) -> Result<(), String> {
    run_account_action(&app, &account_id, AccountAction::Relogin)
}

/// Read-only inheritance status for one account, one row per inheritable subdir.
/// Lists `~/.claude` and the account dir; never writes. Returns all-`none` rows
/// when `~/.claude` is absent.
#[tauri::command]
pub fn get_inherit_status(
    app: AppHandle,
    account_id: String,
) -> Result<Vec<inherit::InheritSubdirStatus>, String> {
    let cfg = Config::load(&paths::config_file_path(&app));
    let account = cfg.account(&account_id).ok_or("unknown account")?;
    let config_dir = expand_tilde(&account.config_dir);
    let source = expand_tilde("~/.claude");
    inherit::inherit_status(&source, &config_dir, &account.inherit_overrides)
        .map_err(|e| e.to_string())
}

/// Persist a Merge/Skip decision for one subdir of one account, then re-apply
/// inheritance so the account dir reflects it. Sticky: the decision is honored
/// on later launches without re-prompting (see `resolve_subdir`).
#[tauri::command]
pub fn set_inherit_decision(
    app: AppHandle,
    account_id: String,
    subdir: String,
    decision: InheritDecision,
) -> Result<(), String> {
    if !inherit::INHERITED_SUBDIRS.contains(&subdir.as_str()) {
        return Err(format!("unknown subdir: {subdir}"));
    }
    let source = expand_tilde("~/.claude");
    let cfg_path = paths::config_file_path(&app);
    let mut cfg = Config::load(&cfg_path);
    let config_dir = expand_tilde(&cfg.account(&account_id).ok_or("unknown account")?.config_dir);

    let account = cfg
        .accounts
        .iter_mut()
        .find(|a| a.id == account_id)
        .ok_or("unknown account")?;
    account.inherit_overrides.insert(subdir, decision);
    let decisions = account.inherit_overrides.clone();
    cfg.save(&cfg_path).map_err(|e| e.to_string())?;

    if source.is_dir() {
        inherit::ensure_inherited(&source, &config_dir, &decisions).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod fallback_tests {
    use super::{
        manual_command, manual_login_command, manual_logout_command, manual_relogin_command,
    };
    #[test]
    fn test_should_build_manual_command_when_spawn_fails() {
        let cmd = manual_command("/home/u/.claude-dino", "/repo");
        assert_eq!(
            cmd,
            "CLAUDE_CONFIG_DIR='/home/u/.claude-dino' sh -c \"cd '/repo' && exec claude\""
        );
    }
    #[test]
    fn test_should_build_manual_login_command_when_spawn_fails() {
        let cmd = manual_login_command("/home/u/.claude-dino");
        assert_eq!(
            cmd,
            "CLAUDE_CONFIG_DIR='/home/u/.claude-dino' sh -c \"exec claude\""
        );
    }
    #[test]
    fn test_should_build_manual_logout_command_when_spawn_fails() {
        let cmd = manual_logout_command("/home/u/.claude-dino");
        assert_eq!(
            cmd,
            "CLAUDE_CONFIG_DIR='/home/u/.claude-dino' sh -c \"claude auth logout\""
        );
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
        let cfg = Config {
            terminal: "iterm".into(),
            ..Config::default()
        };
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
