mod paths;
mod config;
mod launcher;
mod adapters;
mod commands;
mod tray;

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
