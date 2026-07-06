mod adapters;
mod commands;
mod config;
mod inherit;
mod launcher;
mod paths;
mod tray;
mod usage;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .invoke_handler(tauri::generate_handler![
            commands::get_config,
            commands::save_config,
            commands::list_terminals,
            commands::launch_session,
            commands::open_session,
            commands::login_account,
            commands::logout_account,
            commands::relogin_account,
            commands::get_inherit_status,
            commands::set_inherit_decision,
        ])
        .setup(|app| {
            tray::build_tray(app)?;
            use tauri::{Manager, WindowEvent};
            if let Some(win) = app.get_webview_window("main") {
                let win_for_event = win.clone();
                win.on_window_event(move |event| {
                    if let WindowEvent::CloseRequested { api, .. } = event {
                        api.prevent_close();
                        let _ = win_for_event.hide();
                    }
                });
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running claude-multi");
}
