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
