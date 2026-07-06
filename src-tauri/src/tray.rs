use crate::{commands, config::Config, paths, usage};
use tauri::menu::{MenuBuilder, MenuItemBuilder, PredefinedMenuItem, SubmenuBuilder};
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
        ["launch", a, p] => MenuAction::Launch {
            account: a.to_string(),
            project: p.to_string(),
        },
        ["session", a] => MenuAction::Session {
            account: a.to_string(),
        },
        ["login", a] => MenuAction::Login {
            account: a.to_string(),
        },
        ["logout", a] => MenuAction::Logout {
            account: a.to_string(),
        },
        ["relogin", a] => MenuAction::Relogin {
            account: a.to_string(),
        },
        ["prefs"] => MenuAction::Prefs,
        ["quit"] => MenuAction::Quit,
        _ => MenuAction::Unknown,
    }
}

fn build_menu(
    app: &tauri::AppHandle,
    cfg: &Config,
) -> tauri::Result<tauri::menu::Menu<tauri::Wry>> {
    let mut menu = MenuBuilder::new(app);

    // Rolling usage windows (local proxies for the subscription limits, whose
    // real reset lives server-side), computed once per menu build.
    let now = chrono::Utc::now();
    let session_since = usage::session_window_start(now);
    let week_since = usage::week_window_start(now);

    for account in &cfg.accounts {
        let mut sub = SubmenuBuilder::new(app, &account.label);
        match account.logged_in_email() {
            Some(email) => {
                // Logged in: a project-less session, this account's projects, then
                // the email status and the re-login / log out actions.
                sub = sub.item(
                    &MenuItemBuilder::with_id(format!("session::{}", account.id), "New session")
                        .build(app)?,
                );
                for project in cfg.projects.iter().filter(|p| p.account == account.id) {
                    let id = format!("launch::{}::{}", account.id, project.id);
                    sub = sub.item(&MenuItemBuilder::with_id(id, &project.label).build(app)?);
                }
                sub = sub.separator();
                let status_id = format!("status::{}", account.id);
                sub = sub.item(
                    &MenuItemBuilder::with_id(status_id, format!("✓ {email}"))
                        .enabled(false)
                        .build(app)?,
                );
                // Local usage proxies for this account (logged-in only): rolling
                // 5-hour "session" and 7-day "week" windows, each vs its
                // per-account ceiling. Both computed in a single pass over the
                // logs. Disabled/informational, like the status line above.
                let windows = usage::account_usage(account, &[session_since, week_since]);
                sub = sub.item(
                    &MenuItemBuilder::with_id(
                        format!("usage::session::{}", account.id),
                        usage::format_window_line(
                            "Session (5h)",
                            &windows[0],
                            account.usage_limits.session_tokens,
                        ),
                    )
                    .enabled(false)
                    .build(app)?,
                );
                sub = sub.item(
                    &MenuItemBuilder::with_id(
                        format!("usage::week::{}", account.id),
                        usage::format_window_line(
                            "Week (7d)",
                            &windows[1],
                            account.usage_limits.weekly_tokens,
                        ),
                    )
                    .enabled(false)
                    .build(app)?,
                );
                let relogin_id = format!("relogin::{}", account.id);
                sub = sub.item(&MenuItemBuilder::with_id(relogin_id, "Re-login…").build(app)?);
                let logout_id = format!("logout::{}", account.id);
                sub = sub.item(&MenuItemBuilder::with_id(logout_id, "Log out").build(app)?);
            }
            None => {
                // Not logged in: only a login action — sessions and projects need
                // an authenticated account first.
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
    use tauri::tray::{TrayIconBuilder, TrayIconEvent};
    use tauri::Manager;
    let cfg = Config::load(&paths::config_file_path(app.handle()));
    let menu = build_menu(app.handle(), &cfg)?;

    TrayIconBuilder::with_id("main")
        .icon(app.default_window_icon().unwrap().clone())
        .icon_as_template(true)
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(|app, event| {
            let show_err = |msg: String| {
                app.dialog().message(msg).title("claude-multi").show(|_| {});
            };
            match parse_menu_id(event.id().as_ref()) {
                MenuAction::Launch { account, project } => {
                    if let Err(msg) = commands::launch_session(app.clone(), account, project) {
                        show_err(msg);
                    }
                }
                MenuAction::Session { account } => {
                    if let Err(msg) = commands::open_session(app.clone(), account) {
                        show_err(msg);
                    }
                }
                MenuAction::Login { account } => {
                    if let Err(msg) = commands::login_account(app.clone(), account) {
                        show_err(msg);
                    }
                }
                MenuAction::Logout { account } => {
                    if let Err(msg) = commands::logout_account(app.clone(), account) {
                        show_err(msg);
                    }
                }
                MenuAction::Relogin { account } => {
                    if let Err(msg) = commands::relogin_account(app.clone(), account) {
                        show_err(msg);
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
        .on_tray_icon_event(|tray, event| {
            // Rebuild the menu when the cursor enters the icon, so login state
            // (email vs. "Login…") is fresh by the time the menu opens.
            if let TrayIconEvent::Enter { .. } = event {
                let _ = refresh_tray(tray.app_handle());
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
            MenuAction::Launch {
                account: "personal".into(),
                project: "p1".into()
            }
        );
    }

    #[test]
    fn test_should_parse_login_and_static_ids() {
        assert_eq!(
            parse_menu_id("login::dino"),
            MenuAction::Login {
                account: "dino".into()
            }
        );
        assert_eq!(parse_menu_id("prefs"), MenuAction::Prefs);
        assert_eq!(parse_menu_id("quit"), MenuAction::Quit);
        assert_eq!(parse_menu_id("garbage"), MenuAction::Unknown);
    }

    #[test]
    fn test_should_parse_logout_and_relogin_ids() {
        assert_eq!(
            parse_menu_id("logout::dino"),
            MenuAction::Logout {
                account: "dino".into()
            }
        );
        assert_eq!(
            parse_menu_id("relogin::personal"),
            MenuAction::Relogin {
                account: "personal".into()
            }
        );
        assert_eq!(parse_menu_id("status::dino"), MenuAction::Unknown);
        // Usage lines are informational, disabled items (emit no event) — like status.
        assert_eq!(parse_menu_id("usage::session::dino"), MenuAction::Unknown);
        assert_eq!(parse_menu_id("usage::week::dino"), MenuAction::Unknown);
    }

    #[test]
    fn test_should_parse_session_id() {
        assert_eq!(
            parse_menu_id("session::personal"),
            MenuAction::Session {
                account: "personal".into()
            }
        );
    }
}
