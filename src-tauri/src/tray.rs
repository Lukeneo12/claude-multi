use crate::{config::Config, commands, paths};
use tauri::menu::{MenuBuilder, MenuItemBuilder, SubmenuBuilder, PredefinedMenuItem};
use tauri_plugin_dialog::DialogExt;

#[derive(Debug, PartialEq)]
pub enum MenuAction {
    Launch { account: String, project: String },
    Login { account: String },
    Prefs,
    Quit,
    Unknown,
}

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
    use tauri::Manager;
    let cfg = Config::load(&paths::config_file_path(app.handle()));
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
                MenuAction::Login { account } => {
                    if let Err(msg) = commands::login_account(app.clone(), account) {
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
