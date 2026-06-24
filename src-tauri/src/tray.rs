use crate::{config::Config, commands, paths};
use tauri::menu::{MenuBuilder, MenuItemBuilder, SubmenuBuilder, PredefinedMenuItem};
use tauri_plugin_dialog::DialogExt;

#[derive(Debug, PartialEq)]
pub enum MenuAction {
    Launch { account: String, project: String },
    Session { account: String },
    Login { account: String },
    Logout { account: String },
    Relogin { account: String },
    Prefs,
    Quit,
    Unknown,
}

pub fn parse_menu_id(id: &str) -> MenuAction {
    let parts: Vec<&str> = id.split("::").collect();
    match parts.as_slice() {
        ["launch", a, p] => MenuAction::Launch { account: a.to_string(), project: p.to_string() },
        ["session", a] => MenuAction::Session { account: a.to_string() },
        ["login", a] => MenuAction::Login { account: a.to_string() },
        ["logout", a] => MenuAction::Logout { account: a.to_string() },
        ["relogin", a] => MenuAction::Relogin { account: a.to_string() },
        ["prefs"] => MenuAction::Prefs,
        ["quit"] => MenuAction::Quit,
        _ => MenuAction::Unknown,
    }
}

fn build_menu(app: &tauri::AppHandle, cfg: &Config) -> tauri::Result<tauri::menu::Menu<tauri::Wry>> {
    let mut menu = MenuBuilder::new(app);

    for account in &cfg.accounts {
        let mut sub = SubmenuBuilder::new(app, &account.label);
        // Session under this account, outside any project.
        sub = sub.item(&MenuItemBuilder::with_id(format!("session::{}", account.id), "New session").build(app)?);
        // Only this account's projects.
        for project in cfg.projects.iter().filter(|p| p.account == account.id) {
            let id = format!("launch::{}::{}", account.id, project.id);
            sub = sub.item(&MenuItemBuilder::with_id(id, &project.label).build(app)?);
        }
        sub = sub.separator();
        match account.logged_in_email() {
            Some(email) => {
                // Logged in: show the account email as a disabled status line,
                // plus actions to re-login (switch account) or log out.
                let status_id = format!("status::{}", account.id);
                sub = sub.item(
                    &MenuItemBuilder::with_id(status_id, format!("✓ {email}"))
                        .enabled(false)
                        .build(app)?,
                );
                let relogin_id = format!("relogin::{}", account.id);
                sub = sub.item(&MenuItemBuilder::with_id(relogin_id, "Re-login…").build(app)?);
                let logout_id = format!("logout::{}", account.id);
                sub = sub.item(&MenuItemBuilder::with_id(logout_id, "Log out").build(app)?);
            }
            None => {
                let login_id = format!("login::{}", account.id);
                sub = sub.item(&MenuItemBuilder::with_id(login_id, "Login…").build(app)?);
            }
        }
        menu = menu.item(&sub.build()?);
    }

    let prefs = MenuItemBuilder::with_id("prefs", "Preferences…").build(app)?;
    let quit = MenuItemBuilder::with_id("quit", "Quit").build(app)?;
    menu.item(&PredefinedMenuItem::separator(app)?)
        .items(&[&prefs, &quit])
        .build()
}

pub fn build_tray(app: &tauri::App) -> tauri::Result<()> {
    use tauri::tray::TrayIconBuilder;
    use tauri::Manager;
    let cfg = Config::load(&paths::config_file_path(app.handle()));
    let menu = build_menu(app.handle(), &cfg)?;

    TrayIconBuilder::with_id("main")
        .icon(app.default_window_icon().unwrap().clone())
        .icon_as_template(true)
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(|app, event| {
            match parse_menu_id(event.id().as_ref()) {
                MenuAction::Launch { account, project } => {
                    if let Err(msg) = commands::launch_session(app.clone(), account, project) {
                        app.dialog().message(msg).title("claude-multi").show(|_| {});
                    }
                }
                MenuAction::Session { account } => {
                    if let Err(msg) = commands::open_session(app.clone(), account) {
                        app.dialog().message(msg).title("claude-multi").show(|_| {});
                    }
                }
                MenuAction::Login { account } => {
                    if let Err(msg) = commands::login_account(app.clone(), account) {
                        app.dialog().message(msg).title("claude-multi").show(|_| {});
                    }
                }
                MenuAction::Logout { account } => {
                    if let Err(msg) = commands::logout_account(app.clone(), account) {
                        app.dialog().message(msg).title("claude-multi").show(|_| {});
                    }
                }
                MenuAction::Relogin { account } => {
                    if let Err(msg) = commands::relogin_account(app.clone(), account) {
                        app.dialog().message(msg).title("claude-multi").show(|_| {});
                    }
                }
                MenuAction::Prefs => {
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

/// Rebuilds the tray menu from the current config without restarting the app.
/// The previously-registered `on_menu_event` handler keeps working for the new
/// items (it routes by id). Call this after the config changes.
pub fn refresh_tray(app: &tauri::AppHandle) -> tauri::Result<()> {
    let cfg = Config::load(&paths::config_file_path(app));
    let menu = build_menu(app, &cfg)?;
    if let Some(tray) = app.tray_by_id("main") {
        tray.set_menu(Some(menu))?;
    }
    Ok(())
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

    #[test]
    fn test_should_parse_logout_and_relogin_ids() {
        assert_eq!(parse_menu_id("logout::dino"), MenuAction::Logout { account: "dino".into() });
        assert_eq!(parse_menu_id("relogin::personal"), MenuAction::Relogin { account: "personal".into() });
        assert_eq!(parse_menu_id("status::dino"), MenuAction::Unknown);
    }

    #[test]
    fn test_should_parse_session_id() {
        assert_eq!(parse_menu_id("session::personal"), MenuAction::Session { account: "personal".into() });
    }
}
