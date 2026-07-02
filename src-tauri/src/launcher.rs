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

/// Other tools whose global state must be isolated per account, the same way
/// `CLAUDE_CONFIG_DIR` isolates Claude Code's. Each entry is `(ENV_VAR, subdir)`;
/// the exported value is `<config_dir>/<subdir>` (POSIX) or `<config_dir>\<subdir>`
/// (PowerShell). Add an entry here to isolate another tool — no other code changes
/// needed.
const PER_ACCOUNT_ENV_VARS: &[(&str, &str)] = &[("GH_CONFIG_DIR", "gh")];

/// Joins `config_dir` and `subdir` with the path separator for `kind`.
fn join_config_subdir(kind: ScriptKind, config_dir: &str, subdir: &str) -> String {
    match kind {
        ScriptKind::Posix => format!("{config_dir}/{subdir}"),
        ScriptKind::PowerShell => format!("{config_dir}\\{subdir}"),
    }
}

/// Builds the `export VAR='...'` (POSIX) or `$env:VAR = '...'` (PowerShell) lines,
/// one per `PER_ACCOUNT_ENV_VARS` entry, escaped and newline-terminated. Meant to be
/// inserted right after the `CLAUDE_CONFIG_DIR` line and before any command runs.
fn per_account_env_lines(kind: ScriptKind, config_dir: &str) -> String {
    PER_ACCOUNT_ENV_VARS
        .iter()
        .map(|(var, subdir)| {
            let value = join_config_subdir(kind, config_dir, subdir);
            match kind {
                ScriptKind::Posix => {
                    let escaped = posix_single_quote_escape(&value);
                    format!("export {var}='{escaped}'\n")
                }
                ScriptKind::PowerShell => {
                    let escaped = powershell_single_quote_escape(&value);
                    format!("$env:{var} = '{escaped}'\n")
                }
            }
        })
        .collect()
}

pub fn build_script(kind: ScriptKind, config_dir: &str, project_path: &str) -> String {
    let extra_env = per_account_env_lines(kind, config_dir);
    match kind {
        ScriptKind::Posix => {
            let escaped_config = posix_single_quote_escape(config_dir);
            let escaped_project = posix_single_quote_escape(project_path);
            format!(
                "#!/bin/sh\nexport CLAUDE_CONFIG_DIR='{escaped_config}'\n{extra_env}cd '{escaped_project}' || exit 1\nexec claude\n"
            )
        }
        ScriptKind::PowerShell => {
            let escaped_config = powershell_single_quote_escape(config_dir);
            let escaped_project = powershell_single_quote_escape(project_path);
            format!(
                "$env:CLAUDE_CONFIG_DIR = '{escaped_config}'\n{extra_env}Set-Location '{escaped_project}'\nclaude\n"
            )
        }
    }
}

pub fn build_login_script(kind: ScriptKind, config_dir: &str) -> String {
    let extra_env = per_account_env_lines(kind, config_dir);
    match kind {
        ScriptKind::Posix => {
            let escaped_config = posix_single_quote_escape(config_dir);
            format!(
                "#!/bin/sh\nexport CLAUDE_CONFIG_DIR='{escaped_config}'\n{extra_env}exec claude\n"
            )
        }
        ScriptKind::PowerShell => {
            let escaped_config = powershell_single_quote_escape(config_dir);
            format!("$env:CLAUDE_CONFIG_DIR = '{escaped_config}'\n{extra_env}claude\n")
        }
    }
}

pub fn build_logout_script(kind: ScriptKind, config_dir: &str) -> String {
    let extra_env = per_account_env_lines(kind, config_dir);
    match kind {
        ScriptKind::Posix => {
            let escaped_config = posix_single_quote_escape(config_dir);
            format!(
                "#!/bin/sh\nexport CLAUDE_CONFIG_DIR='{escaped_config}'\n{extra_env}claude auth logout\necho\necho 'You can close this window.'\n"
            )
        }
        ScriptKind::PowerShell => {
            let escaped_config = powershell_single_quote_escape(config_dir);
            format!(
                "$env:CLAUDE_CONFIG_DIR = '{escaped_config}'\n{extra_env}claude auth logout\nWrite-Host ''\nWrite-Host 'You can close this window.'\n"
            )
        }
    }
}

pub fn build_relogin_script(kind: ScriptKind, config_dir: &str) -> String {
    let extra_env = per_account_env_lines(kind, config_dir);
    match kind {
        ScriptKind::Posix => {
            let escaped_config = posix_single_quote_escape(config_dir);
            format!(
                "#!/bin/sh\nexport CLAUDE_CONFIG_DIR='{escaped_config}'\n{extra_env}claude auth logout\nexec claude auth login\n"
            )
        }
        ScriptKind::PowerShell => {
            let escaped_config = powershell_single_quote_escape(config_dir);
            format!(
                "$env:CLAUDE_CONFIG_DIR = '{escaped_config}'\n{extra_env}claude auth logout\nclaude auth login\n"
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

    #[test]
    fn test_should_export_gh_config_dir_when_build_script_posix() {
        let s = build_script(ScriptKind::Posix, "/home/u/.claude-dino", "/repo/app");
        assert!(s.contains("export GH_CONFIG_DIR='/home/u/.claude-dino/gh'"));
    }

    #[test]
    fn test_should_export_gh_config_dir_before_cd_when_build_script_posix() {
        let s = build_script(ScriptKind::Posix, "/home/u/.claude-dino", "/repo/app");
        let claude_line = s.find("CLAUDE_CONFIG_DIR").unwrap();
        let gh_line = s.find("GH_CONFIG_DIR").unwrap();
        let cd_line = s.find("cd '").unwrap();
        assert!(
            claude_line < gh_line && gh_line < cd_line,
            "GH_CONFIG_DIR must be exported after CLAUDE_CONFIG_DIR and before cd"
        );
    }

    #[test]
    fn test_should_export_gh_config_dir_when_build_script_powershell() {
        let s = build_script(
            ScriptKind::PowerShell,
            r"C:\Users\u\.claude-dino",
            r"C:\repo\app",
        );
        assert!(s.contains(r"$env:GH_CONFIG_DIR = 'C:\Users\u\.claude-dino\gh'"));
    }

    #[test]
    fn test_should_export_gh_config_dir_when_login_script_posix() {
        let s = build_login_script(ScriptKind::Posix, "/home/u/.claude-personal");
        assert!(s.contains("export GH_CONFIG_DIR='/home/u/.claude-personal/gh'"));
        let gh_line = s.find("GH_CONFIG_DIR").unwrap();
        let exec_line = s.find("exec claude").unwrap();
        assert!(
            gh_line < exec_line,
            "GH_CONFIG_DIR must be set before exec claude"
        );
    }

    #[test]
    fn test_should_export_gh_config_dir_when_login_script_powershell() {
        let s = build_login_script(ScriptKind::PowerShell, r"C:\Users\u\.claude-personal");
        assert!(s.contains(r"$env:GH_CONFIG_DIR = 'C:\Users\u\.claude-personal\gh'"));
    }

    #[test]
    fn test_should_export_gh_config_dir_when_logout_script_posix() {
        let s = build_logout_script(ScriptKind::Posix, "/home/u/.claude-dino");
        assert!(s.contains("export GH_CONFIG_DIR='/home/u/.claude-dino/gh'"));
        let gh_line = s.find("GH_CONFIG_DIR").unwrap();
        let logout_line = s.find("claude auth logout").unwrap();
        assert!(
            gh_line < logout_line,
            "GH_CONFIG_DIR must be set before claude auth logout"
        );
    }

    #[test]
    fn test_should_export_gh_config_dir_when_logout_script_powershell() {
        let s = build_logout_script(ScriptKind::PowerShell, r"C:\Users\u\.claude-dino");
        assert!(s.contains(r"$env:GH_CONFIG_DIR = 'C:\Users\u\.claude-dino\gh'"));
    }

    #[test]
    fn test_should_export_gh_config_dir_when_relogin_script_posix() {
        let s = build_relogin_script(ScriptKind::Posix, "/home/u/.claude-dino");
        assert!(s.contains("export GH_CONFIG_DIR='/home/u/.claude-dino/gh'"));
        let gh_line = s.find("GH_CONFIG_DIR").unwrap();
        let logout_line = s.find("claude auth logout").unwrap();
        assert!(
            gh_line < logout_line,
            "GH_CONFIG_DIR must be set before claude auth logout"
        );
    }

    #[test]
    fn test_should_export_gh_config_dir_when_relogin_script_powershell() {
        let s = build_relogin_script(ScriptKind::PowerShell, r"C:\Users\u\.claude-dino");
        assert!(s.contains(r"$env:GH_CONFIG_DIR = 'C:\Users\u\.claude-dino\gh'"));
    }

    #[test]
    fn test_should_escape_single_quote_in_gh_config_dir_when_posix() {
        let s = build_script(ScriptKind::Posix, "/home/o'brien/.claude", "/repo/app");
        assert!(s.contains("GH_CONFIG_DIR='/home/o'\\''brien/.claude/gh'"));
    }

    #[test]
    fn test_should_escape_single_quote_in_gh_config_dir_when_powershell() {
        let s = build_script(
            ScriptKind::PowerShell,
            r"C:\Users\o'brien\.claude",
            r"C:\repo\app",
        );
        assert!(s.contains("GH_CONFIG_DIR = 'C:\\Users\\o''brien\\.claude\\gh'"));
    }
}
