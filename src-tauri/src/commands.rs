use crate::config::{
    apply_inherit_decisions, merge_inherit_overrides, Config, ConfigStore, InheritDecision,
};
use crate::paths::expand_tilde;
use crate::{adapters, inherit, launcher, session};
use serde::Serialize;
use std::collections::HashMap;
use tauri::{AppHandle, Manager};
use tauri_plugin_clipboard_manager::ClipboardExt;

/// Strict config load through the process-wide store: a corrupt `config.json`
/// is an error (and is backed up), never silently replaced by defaults.
fn load_config(app: &AppHandle) -> Result<Config, String> {
    app.state::<ConfigStore>().load()
}

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

    // Lock-free read: the config lock must never be held across the blocking
    // prompt below (sync commands and tray events share the main thread and
    // `Mutex` is not reentrant).
    let cfg = load_config(app)?;

    let config_dir = expand_tilde(&cfg.account(account_id).ok_or("unknown account")?.config_dir);
    let decisions = cfg
        .account(account_id)
        .map(|a| a.inherit_overrides.clone())
        .unwrap_or_default();

    // Best-effort: seeding settings.json is a convenience and must never block
    // the session launch — its warning prints even when the inherit pass fails.
    let outcome = match inherit::seed_and_apply(&source, &config_dir, &decisions) {
        Ok(outcome) => outcome,
        Err(e) => {
            if let Some(s) = e.seed_error {
                eprintln!("settings.json seed failed for account '{account_id}': {s}");
            }
            return Err(e.inherit_error.to_string());
        }
    };
    if let Some(e) = outcome.seed_error {
        eprintln!("settings.json seed failed for account '{account_id}': {e}");
    }
    if outcome.needs_prompt.is_empty() {
        return Ok(());
    }

    // Prompt once per conflicted subdir (no lock held), then persist only the
    // new decisions through the store so concurrent edits are merged, not lost.
    let mut prompted = HashMap::new();
    for sub in &outcome.needs_prompt {
        prompted.insert(sub.clone(), prompt_inherit_decision(app, account_id, sub));
    }
    let merged = app
        .state::<ConfigStore>()
        .update(|c| apply_inherit_decisions(c, account_id, &prompted))?;

    inherit::ensure_inherited(&source, &config_dir, &merged).map_err(|e| e.to_string())?;
    Ok(())
}

/// Checks that a project's directory still exists before any launch work.
/// `spawn` runs the terminal with `current_dir` set to it, so a missing dir
/// makes the OS spawn fail — which used to be misreported as a terminal
/// problem ("Couldn't open terminal …") when the terminal was fine.
fn validate_project_dir(label: &str, path: &std::path::Path) -> Result<(), String> {
    if path.is_dir() {
        Ok(())
    } else {
        Err(format!(
            "Project '{label}' points to '{}', which doesn't exist. Fix its path in Preferences.",
            path.display()
        ))
    }
}

#[derive(Serialize)]
pub struct TerminalInfo {
    pub id: String,
    pub label: String,
}

#[tauri::command]
pub fn get_config(app: AppHandle) -> Result<Config, String> {
    load_config(&app)
}

#[tauri::command]
pub fn save_config(app: AppHandle, config: Config) -> Result<(), String> {
    // Reject before taking the store lock: pure check, nothing to persist yet.
    crate::config::validate_unique_config_dirs(&config)?;
    // Preferences never edits inherit_overrides: keep whatever is on disk (a
    // tray-prompt decision may have landed since the UI loaded its copy).
    app.state::<ConfigStore>().update(|disk| {
        *disk = merge_inherit_overrides(config, disk);
        Ok(())
    })?;
    // Rebuild (after the lock is released) the tray menu so changes apply immediately, no restart needed.
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
    let cfg = load_config(&app)?;
    let account = cfg.account(&account_id).ok_or("unknown account")?;
    let project = cfg.project(&project_id).ok_or("unknown project")?;
    let adapter = adapters::find_adapter(&cfg.terminal).ok_or("unknown terminal")?;

    let project_path = expand_tilde(&project.path);
    validate_project_dir(&project.label, &project_path)?;
    session::ensure_session_usable(account)?;

    ensure_account_inherits(&app, &account_id)?;

    let config_dir = expand_tilde(&account.config_dir);
    let cd = config_dir.to_string_lossy();
    let pp = project_path.to_string_lossy();

    let script = launcher::build_script(adapter.kind, &cd, &pp);
    let script_path = launcher::write_script(&script, adapter.kind).map_err(|e| e.to_string())?;

    adapters::spawn(&adapter, &script_path.to_string_lossy(), &pp).map_err(|e| {
        let _ = app.clipboard().write_text(launcher::manual_command(adapter.kind, &cd, &pp));
        format!(
            "Couldn't open terminal '{}' ({e}). The launch command was copied to your clipboard — paste it into any terminal.",
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
    let cfg = load_config(app)?;
    let account = cfg.account(account_id).ok_or("unknown account")?;
    let adapter = adapters::find_adapter(&cfg.terminal).ok_or("unknown terminal")?;

    // A plain session needs live credentials; login/re-login/logout are exactly
    // how an expired account gets fixed, so they must stay available.
    if matches!(action, AccountAction::Session) {
        session::ensure_session_usable(account)?;
    }

    let config_dir = expand_tilde(&account.config_dir);
    let cd = config_dir.to_string_lossy();

    // The action is about to change this account's credentials in the terminal:
    // drop the cached verdict so the next menu build re-reads them instead of
    // serving the pre-action state for the rest of the TTL.
    if matches!(
        action,
        AccountAction::Login | AccountAction::Logout | AccountAction::Relogin
    ) {
        session::invalidate_verdict(&config_dir);
    }

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
            launcher::manual_login_command(adapter.kind, &cd),
            "login",
        ),
        AccountAction::Session => (
            launcher::build_login_script(adapter.kind, &cd),
            launcher::manual_login_command(adapter.kind, &cd),
            "the session",
        ),
        AccountAction::Logout => (
            launcher::build_logout_script(adapter.kind, &cd),
            launcher::manual_logout_command(adapter.kind, &cd),
            "logout",
        ),
        AccountAction::Relogin => (
            launcher::build_relogin_script(adapter.kind, &cd),
            launcher::manual_relogin_command(adapter.kind, &cd),
            "re-login",
        ),
    };

    let script_path = launcher::write_script(&script, adapter.kind).map_err(|e| e.to_string())?;
    adapters::spawn(&adapter, &script_path.to_string_lossy(), &cd).map_err(|e| {
        let _ = app.clipboard().write_text(fallback);
        format!(
            "Couldn't open terminal '{}' for {what} ({e}). The command was copied to your clipboard — paste it into any terminal.",
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
    let cfg = load_config(&app)?;
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
    let new = HashMap::from([(subdir, decision)]);
    let (config_dir, decisions) = app.state::<ConfigStore>().update(|c| {
        let decisions = apply_inherit_decisions(c, &account_id, &new)?;
        let dir = c
            .account(&account_id)
            .ok_or("unknown account")?
            .config_dir
            .clone();
        Ok((expand_tilde(&dir), decisions))
    })?;

    if source.is_dir() {
        inherit::ensure_inherited(&source, &config_dir, &decisions).map_err(|e| e.to_string())?;
    }
    Ok(())
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
mod validate_project_dir_tests {
    use super::validate_project_dir;

    #[test]
    fn test_should_accept_project_when_dir_exists() {
        let dir = tempfile::tempdir().unwrap();
        assert!(validate_project_dir("app", dir.path()).is_ok());
    }

    #[test]
    fn test_should_name_project_and_path_when_dir_missing() {
        let missing = std::path::Path::new("/nonexistent/cm-project");
        let err = validate_project_dir("my-backend", missing).unwrap_err();
        assert!(err.contains("my-backend"));
        assert!(err.contains("/nonexistent/cm-project"));
        assert!(!err.contains("terminal"), "must not blame the terminal");
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
