use std::path::PathBuf;

pub fn expand_tilde(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")) {
            return PathBuf::from(home).join(rest);
        }
    }
    PathBuf::from(path)
}

pub fn config_file_path(app: &tauri::AppHandle) -> PathBuf {
    use tauri::Manager;
    app.path()
        .app_config_dir()
        .expect("app_config_dir unavailable")
        .join("config.json")
}

/// Bundle identifier — must match `identifier` in `tauri.conf.json`. Tauri
/// derives `app_config_dir` from it, and the standalone resolver below must
/// agree with that path or the CLI would read a different config.
pub const APP_IDENTIFIER: &str = "com.lucasdonadio.claude-multi";

/// Tauri-free equivalent of `config_file_path`, for the `cms` CLI bin.
/// `dirs::config_dir()` matches Tauri's `app_config_dir` base on all three
/// desktop OSes: macOS `~/Library/Application Support`, Linux XDG config dir,
/// Windows Roaming AppData.
pub fn standalone_config_file_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join(APP_IDENTIFIER).join("config.json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_expand_leading_tilde_when_path_starts_with_tilde_slash() {
        let home = std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .unwrap();
        assert_eq!(
            expand_tilde("~/.claude-personal"),
            PathBuf::from(home).join(".claude-personal")
        );
    }

    #[test]
    fn test_should_return_path_unchanged_when_no_leading_tilde() {
        assert_eq!(expand_tilde("/abs/path"), PathBuf::from("/abs/path"));
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn test_should_match_tauri_app_config_dir_when_on_macos() {
        let home = std::env::var("HOME").unwrap();
        assert_eq!(
            standalone_config_file_path().unwrap(),
            PathBuf::from(home)
                .join("Library/Application Support")
                .join("com.lucasdonadio.claude-multi")
                .join("config.json")
        );
    }

    #[test]
    fn test_should_match_tauri_conf_identifier_when_comparing_app_identifier() {
        let conf: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        assert_eq!(
            conf["identifier"].as_str().unwrap(),
            APP_IDENTIFIER,
            "APP_IDENTIFIER must match `identifier` in tauri.conf.json — the CLI resolves its config path from it"
        );
    }

    #[test]
    fn test_should_end_with_identifier_and_filename_when_resolving_standalone_path() {
        let p = standalone_config_file_path().unwrap();
        assert!(p.ends_with(format!("{APP_IDENTIFIER}/config.json")));
    }
}
