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
}
