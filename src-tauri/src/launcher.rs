use std::io::Write;
use std::path::PathBuf;

#[derive(Clone, Copy)]
pub enum ScriptKind {
    Posix,
    PowerShell,
}

pub fn build_script(kind: ScriptKind, config_dir: &str, project_path: &str) -> String {
    match kind {
        ScriptKind::Posix => format!(
            "#!/bin/sh\nexport CLAUDE_CONFIG_DIR='{config_dir}'\ncd '{project_path}' || exit 1\nexec claude\n"
        ),
        ScriptKind::PowerShell => format!(
            "$env:CLAUDE_CONFIG_DIR = '{config_dir}'\nSet-Location '{project_path}'\nclaude\n"
        ),
    }
}

pub fn build_login_script(kind: ScriptKind, config_dir: &str) -> String {
    match kind {
        ScriptKind::Posix => format!(
            "#!/bin/sh\nexport CLAUDE_CONFIG_DIR='{config_dir}'\nexec claude\n"
        ),
        ScriptKind::PowerShell => format!(
            "$env:CLAUDE_CONFIG_DIR = '{config_dir}'\nclaude\n"
        ),
    }
}

pub fn write_script(content: &str, kind: ScriptKind) -> std::io::Result<PathBuf> {
    let ext = match kind {
        ScriptKind::Posix => "sh",
        ScriptKind::PowerShell => "ps1",
    };
    // Unique-enough name without Date/random: use process id + content length.
    let name = format!("claude-multi-{}-{}.{}", std::process::id(), content.len(), ext);
    let path = std::env::temp_dir().join(name);
    let mut f = std::fs::File::create(&path)?;
    f.write_all(content.as_bytes())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))?;
    }
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
        let s = build_script(ScriptKind::PowerShell, r"C:\Users\u\.claude-dino", r"C:\repo\app");
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
}
