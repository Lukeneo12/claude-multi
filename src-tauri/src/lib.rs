use tauri::{
    menu::{MenuBuilder, MenuItemBuilder},
    tray::TrayIconBuilder,
    Manager,
};

mod paths;
mod config;
mod launcher;
mod adapters;
mod commands;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            commands::get_config,
            commands::save_config,
            commands::list_terminals,
            commands::launch_session,
            commands::login_account,
        ])
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
