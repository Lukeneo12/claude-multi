use crate::launcher::ScriptKind;
use std::process::Command;

#[derive(Clone)]
pub struct TerminalAdapter {
    pub id: String,
    pub label: String,
    pub command: String,
    pub args: Vec<String>,
    pub kind: ScriptKind,
}

pub fn render_args(args: &[String], script: &str, cwd: &str) -> Vec<String> {
    args.iter()
        .map(|a| a.replace("{{script}}", script).replace("{{cwd}}", cwd))
        .collect()
}

fn adapter(id: &str, label: &str, command: &str, args: &[&str], kind: ScriptKind) -> TerminalAdapter {
    TerminalAdapter {
        id: id.into(),
        label: label.into(),
        command: command.into(),
        args: args.iter().map(|s| s.to_string()).collect(),
        kind,
    }
}

pub fn builtin_adapters() -> Vec<TerminalAdapter> {
    // `open -a <App> <script>` on macOS opens the app AND runs the script file.
    // Warp is the known spike (see spec Risks): verify it actually runs the script.
    let mut v = vec![];
    #[cfg(target_os = "macos")]
    {
        v.push(adapter("terminal", "Terminal.app", "open", &["-a", "Terminal", "{{script}}"], ScriptKind::Posix));
        v.push(adapter("iterm", "iTerm2", "open", &["-a", "iTerm", "{{script}}"], ScriptKind::Posix));
        v.push(adapter("warp", "Warp (verify)", "open", &["-a", "Warp", "{{script}}"], ScriptKind::Posix));
    }
    #[cfg(target_os = "linux")]
    {
        v.push(adapter("gnome-terminal", "GNOME Terminal", "gnome-terminal", &["--", "sh", "{{script}}"], ScriptKind::Posix));
        v.push(adapter("konsole", "Konsole", "konsole", &["-e", "sh", "{{script}}"], ScriptKind::Posix));
    }
    #[cfg(target_os = "windows")]
    {
        v.push(adapter("wt", "Windows Terminal", "wt.exe", &["powershell", "-NoExit", "-File", "{{script}}"], ScriptKind::PowerShell));
        v.push(adapter("powershell", "PowerShell", "powershell.exe", &["-NoExit", "-File", "{{script}}"], ScriptKind::PowerShell));
    }
    v
}

pub fn find_adapter(id: &str) -> Option<TerminalAdapter> {
    builtin_adapters().into_iter().find(|a| a.id == id)
}

pub fn spawn(adapter: &TerminalAdapter, script_path: &str, cwd: &str) -> std::io::Result<()> {
    let args = render_args(&adapter.args, script_path, cwd);
    Command::new(&adapter.command).args(&args).spawn()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_substitute_script_and_cwd_placeholders_when_rendering() {
        let tmpl = vec!["-a".to_string(), "Terminal".to_string(), "{{script}}".to_string()];
        let out = render_args(&tmpl, "/tmp/s.sh", "/repo");
        assert_eq!(out, vec!["-a", "Terminal", "/tmp/s.sh"]);
    }

    #[test]
    fn test_should_substitute_cwd_placeholder_when_present() {
        let tmpl = vec!["--working-directory={{cwd}}".to_string(), "{{script}}".to_string()];
        let out = render_args(&tmpl, "/tmp/s.sh", "/repo");
        assert_eq!(out, vec!["--working-directory=/repo", "/tmp/s.sh"]);
    }

    #[test]
    fn test_should_find_builtin_adapter_by_id() {
        assert!(find_adapter("terminal").is_some() || find_adapter("gnome-terminal").is_some() || find_adapter("wt").is_some());
        assert!(find_adapter("does-not-exist").is_none());
    }
}
