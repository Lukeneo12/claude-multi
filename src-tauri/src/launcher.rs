use std::io::Write;
use std::path::PathBuf;

#[derive(Clone, Copy)]
pub enum ScriptKind {
    Posix,
    #[cfg_attr(not(target_os = "windows"), allow(dead_code))]
    PowerShell,
}

/// Escapes single quotes for POSIX shell single-quoted strings.
/// Each `'` in the value becomes `'\''` (close quote, escaped literal, reopen quote).
fn posix_single_quote_escape(s: &str) -> String {
    s.replace('\'', "'\\''")
}

/// Escapes single quotes for PowerShell single-quoted strings.
/// Each `'` in the value becomes `''` (doubled single quote).
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn powershell_single_quote_escape(s: &str) -> String {
    s.replace('\'', "''")
}

pub fn build_script(kind: ScriptKind, config_dir: &str, project_path: &str) -> String {
    match kind {
        ScriptKind::Posix => {
            let escaped_config = posix_single_quote_escape(config_dir);
            let escaped_project = posix_single_quote_escape(project_path);
            format!(
                "#!/bin/sh\nexport CLAUDE_CONFIG_DIR='{escaped_config}'\ncd '{escaped_project}' || exit 1\nexec claude\n"
            )
        }
        ScriptKind::PowerShell => {
            let escaped_config = powershell_single_quote_escape(config_dir);
            let escaped_project = powershell_single_quote_escape(project_path);
            format!(
                "$env:CLAUDE_CONFIG_DIR = '{escaped_config}'\nSet-Location '{escaped_project}'\nclaude\n"
            )
        }
    }
}

pub fn build_login_script(kind: ScriptKind, config_dir: &str) -> String {
    match kind {
        ScriptKind::Posix => {
            let escaped_config = posix_single_quote_escape(config_dir);
            format!("#!/bin/sh\nexport CLAUDE_CONFIG_DIR='{escaped_config}'\nexec claude\n")
        }
        ScriptKind::PowerShell => {
            let escaped_config = powershell_single_quote_escape(config_dir);
            format!("$env:CLAUDE_CONFIG_DIR = '{escaped_config}'\nclaude\n")
        }
    }
}

pub fn build_logout_script(kind: ScriptKind, config_dir: &str) -> String {
    match kind {
        ScriptKind::Posix => {
            let escaped_config = posix_single_quote_escape(config_dir);
            format!(
                "#!/bin/sh\nexport CLAUDE_CONFIG_DIR='{escaped_config}'\nclaude auth logout\necho\necho 'You can close this window.'\n"
            )
        }
        ScriptKind::PowerShell => {
            let escaped_config = powershell_single_quote_escape(config_dir);
            format!(
                "$env:CLAUDE_CONFIG_DIR = '{escaped_config}'\nclaude auth logout\nWrite-Host ''\nWrite-Host 'You can close this window.'\n"
            )
        }
    }
}

pub fn build_relogin_script(kind: ScriptKind, config_dir: &str) -> String {
    match kind {
        ScriptKind::Posix => {
            let escaped_config = posix_single_quote_escape(config_dir);
            format!(
                "#!/bin/sh\nexport CLAUDE_CONFIG_DIR='{escaped_config}'\nclaude auth logout\nexec claude auth login\n"
            )
        }
        ScriptKind::PowerShell => {
            let escaped_config = powershell_single_quote_escape(config_dir);
            format!(
                "$env:CLAUDE_CONFIG_DIR = '{escaped_config}'\nclaude auth logout\nclaude auth login\n"
            )
        }
    }
}

pub fn write_script(content: &str, kind: ScriptKind) -> std::io::Result<PathBuf> {
    let ext = match kind {
        ScriptKind::Posix => "sh",
        ScriptKind::PowerShell => "ps1",
    };
    let suffix = format!(".{ext}");
    let mut builder = tempfile::Builder::new();
    builder.prefix("claude-multi-").suffix(&suffix);
    let mut f = builder.tempfile()?; // random name, 0600 on unix, atomic create_new
    f.write_all(content.as_bytes())?;
    f.flush()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        f.as_file()
            .set_permissions(std::fs::Permissions::from_mode(0o700))?;
    }
    let (_file, path) = f.keep().map_err(std::io::Error::other)?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_export_config_dir_and_exec_claude_when_posix() {
        let s = build_script(ScriptKind::Posix, "/home/u/.claude-dino", "/repo/app");
        assert!(s.contains("export CLAUDE_CONFIG_DIR='/home/u/.claude-dino'"));
        assert!(s.contains("cd '/repo/app'"));
        assert!(s.trim_end().ends_with("exec claude"));
    }

    #[test]
    fn test_should_set_env_and_run_claude_when_powershell() {
        let s = build_script(
            ScriptKind::PowerShell,
            r"C:\Users\u\.claude-dino",
            r"C:\repo\app",
        );
        assert!(s.contains(r"$env:CLAUDE_CONFIG_DIR = 'C:\Users\u\.claude-dino'"));
        assert!(s.contains(r"Set-Location 'C:\repo\app'"));
        assert!(s.contains("claude"));
    }

    #[test]
    fn test_should_not_cd_into_project_when_login_script() {
        let s = build_login_script(ScriptKind::Posix, "/home/u/.claude-personal");
        assert!(s.contains("export CLAUDE_CONFIG_DIR='/home/u/.claude-personal'"));
        assert!(!s.contains("cd '"));
        assert!(s.trim_end().ends_with("exec claude"));
    }

    #[test]
    fn test_should_escape_single_quote_in_config_dir_when_posix() {
        let s = build_script(ScriptKind::Posix, "/home/o'brien/.claude", "/repo/app");
        assert!(s.contains("CLAUDE_CONFIG_DIR='/home/o'\\''brien/.claude'"));
    }

    #[test]
    fn test_should_escape_single_quote_when_powershell() {
        let s = build_script(
            ScriptKind::PowerShell,
            r"C:\Users\u\.claude",
            r"C:\Users\o'brien\repo",
        );
        assert!(s.contains("Set-Location 'C:\\Users\\o''brien\\repo'"));
    }

    #[test]
    fn test_should_run_auth_logout_when_logout_script() {
        let s = build_logout_script(ScriptKind::Posix, "/home/u/.claude-dino");
        assert!(s.contains("export CLAUDE_CONFIG_DIR='/home/u/.claude-dino'"));
        assert!(s.contains("claude auth logout"));
        assert!(!s.contains("cd '"));
    }

    #[test]
    fn test_should_logout_then_login_when_relogin_script() {
        let s = build_relogin_script(ScriptKind::Posix, "/home/u/.claude-dino");
        assert!(s.contains("export CLAUDE_CONFIG_DIR='/home/u/.claude-dino'"));
        let logout = s.find("claude auth logout").unwrap();
        let login = s.find("claude auth login").unwrap();
        assert!(logout < login, "logout must run before login");
        assert!(s.trim_end().ends_with("exec claude auth login"));
    }
}
