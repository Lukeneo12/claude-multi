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

fn adapter(
    id: &str,
    label: &str,
    command: &str,
    args: &[&str],
    kind: ScriptKind,
) -> TerminalAdapter {
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
    // Verified for Terminal, iTerm2 and Warp.
    let mut v = vec![];
    #[cfg(target_os = "macos")]
    {
        v.push(adapter(
            "terminal",
            "Terminal.app",
            "open",
            &["-a", "Terminal", "{{script}}"],
            ScriptKind::Posix,
        ));
        v.push(adapter(
            "iterm",
            "iTerm2",
            "open",
            &["-a", "iTerm", "{{script}}"],
            ScriptKind::Posix,
        ));
        v.push(adapter(
            "warp",
            "Warp",
            "open",
            &["-a", "Warp", "{{script}}"],
            ScriptKind::Posix,
        ));
    }
    #[cfg(target_os = "linux")]
    {
        v.push(adapter(
            "gnome-terminal",
            "GNOME Terminal",
            "gnome-terminal",
            &["--", "sh", "{{script}}"],
            ScriptKind::Posix,
        ));
        v.push(adapter(
            "konsole",
            "Konsole",
            "konsole",
            &["-e", "sh", "{{script}}"],
            ScriptKind::Posix,
        ));
    }
    #[cfg(target_os = "windows")]
    v.extend(windows_adapters());
    v
}

/// `powershell.exe` flags shared by every Windows adapter. `-ExecutionPolicy
/// Bypass` applies to this process only (never persisted): stock Windows
/// clients default to `Restricted`, which refuses to run any `.ps1`, including
/// our own temp script. A Group Policy-enforced policy still wins and can't be
/// overridden from the command line. `-File` must come last: anything after it
/// is passed to the script.
///
/// The flag works by setting `PSExecutionPolicyPreference=Bypass`, which
/// PowerShell re-reads on every script load and child processes inherit. The
/// launch script deliberately leaves it set: `claude` installed via npm is a
/// `claude.ps1` shim that `Restricted` would block in the same window, as would
/// any script the user runs in the `-NoExit` session afterwards.
const POWERSHELL_SCRIPT_ARGS: &[&str] = &[
    "-NoExit",
    "-ExecutionPolicy",
    "Bypass",
    "-File",
    "{{script}}",
];

/// The Windows adapters, defined outside the `#[cfg]` so their command lines
/// are unit-tested on every OS.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub(crate) fn windows_adapters() -> Vec<TerminalAdapter> {
    let wt_args: Vec<&str> = std::iter::once("powershell")
        .chain(POWERSHELL_SCRIPT_ARGS.iter().copied())
        .collect();
    vec![
        adapter(
            "wt",
            "Windows Terminal",
            "wt.exe",
            &wt_args,
            ScriptKind::PowerShell,
        ),
        adapter(
            "powershell",
            "PowerShell",
            "powershell.exe",
            POWERSHELL_SCRIPT_ARGS,
            ScriptKind::PowerShell,
        ),
    ]
}

pub fn find_adapter(id: &str) -> Option<TerminalAdapter> {
    builtin_adapters().into_iter().find(|a| a.id == id)
}

pub fn spawn(adapter: &TerminalAdapter, script_path: &str, cwd: &str) -> std::io::Result<()> {
    let args = render_args(&adapter.args, script_path, cwd);
    Command::new(&adapter.command)
        .current_dir(cwd)
        .args(&args)
        .spawn()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_substitute_script_and_cwd_placeholders_when_rendering() {
        let tmpl = vec![
            "-a".to_string(),
            "Terminal".to_string(),
            "{{script}}".to_string(),
        ];
        let out = render_args(&tmpl, "/tmp/s.sh", "/repo");
        assert_eq!(out, vec!["-a", "Terminal", "/tmp/s.sh"]);
    }

    #[test]
    fn test_should_substitute_cwd_placeholder_when_present() {
        let tmpl = vec![
            "--working-directory={{cwd}}".to_string(),
            "{{script}}".to_string(),
        ];
        let out = render_args(&tmpl, "/tmp/s.sh", "/repo");
        assert_eq!(out, vec!["--working-directory=/repo", "/tmp/s.sh"]);
    }

    #[test]
    fn test_should_bypass_execution_policy_when_windows_adapters_run_a_script() {
        // Stock Windows clients default to `Restricted`, which refuses any .ps1.
        for a in windows_adapters() {
            let args: Vec<&str> = a.args.iter().map(String::as_str).collect();
            let file = args.iter().position(|x| *x == "-File").expect("-File");
            let pos = args
                .windows(2)
                .position(|w| w == ["-ExecutionPolicy", "Bypass"])
                .unwrap_or_else(|| panic!("{} lacks -ExecutionPolicy Bypass: {args:?}", a.id));
            assert!(pos < file, "{}: flags after -File go to the script", a.id);
            assert_eq!(
                args.last(),
                Some(&"{{script}}"),
                "{}: -File must be last",
                a.id
            );
            assert!(args.contains(&"-NoExit"), "{}: keep the window open", a.id);
        }
    }

    #[test]
    fn test_should_launch_the_same_powershell_flags_when_using_windows_terminal() {
        let all = windows_adapters();
        let wt = all.iter().find(|a| a.id == "wt").unwrap();
        let ps = all.iter().find(|a| a.id == "powershell").unwrap();
        assert_eq!(wt.args[0], "powershell");
        assert_eq!(wt.args[1..], ps.args[..]);
    }

    #[test]
    fn test_should_find_builtin_adapter_by_id() {
        assert!(
            find_adapter("terminal").is_some()
                || find_adapter("gnome-terminal").is_some()
                || find_adapter("wt").is_some()
        );
        assert!(find_adapter("does-not-exist").is_none());
    }
}
