//! `cms` — launch a per-account Claude Code session inline in the current
//! terminal (for IDE terminals where the tray's new-window launch doesn't
//! fit). Edge I/O only; decision logic lives in `claude_multi_lib::cli`.

use claude_multi_lib::cli::{
    launch_plan, match_account, parse_cli_args, AccountMatchError, CliCommand,
};
use claude_multi_lib::config::Config;
use claude_multi_lib::{inherit, paths, session};
use std::process::ExitCode;

fn usage() -> String {
    "Usage: cms <account> [claude args...]   launch claude here under <account>\n       cms --list | -l                  list configured accounts\n       cms --help | -h                  show this help\n\n<account> matches account id or label, case-insensitively, by unique prefix."
        .to_string()
}

fn account_list(cfg: &Config) -> String {
    cfg.accounts
        .iter()
        .map(|a| {
            let auth = a
                .logged_in_email()
                .unwrap_or_else(|| "not logged in".to_string());
            format!("  {:<12} {:<16} {auth}", a.id, a.label)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn load_config() -> Result<Config, String> {
    let path = paths::standalone_config_file_path()
        .ok_or("could not resolve the user config directory")?;
    if !path.is_file() {
        return Err(format!(
            "claude-multi is not configured yet (missing {}).\nOpen the claude-multi app once to create your accounts.",
            path.display()
        ));
    }
    // Strict load: a corrupt config must not fall back to the default
    // `personal` account, or the CLI would create ~/.claude-personal state
    // the user never configured (see spec).
    Config::try_load(&path)
        .map_err(|e| format!("could not read {}: {e}", path.display()))
}

fn run() -> Result<ExitCode, String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match parse_cli_args(&args) {
        CliCommand::Help { explicit } => {
            let text = match load_config() {
                Ok(cfg) => format!("{}\n\nAccounts:\n{}", usage(), account_list(&cfg)),
                Err(_) => usage(),
            };
            if explicit {
                println!("{text}");
                Ok(ExitCode::SUCCESS)
            } else {
                eprintln!("{text}");
                Ok(ExitCode::from(2))
            }
        }
        CliCommand::List => {
            let cfg = load_config()?;
            println!("{}", account_list(&cfg));
            Ok(ExitCode::SUCCESS)
        }
        CliCommand::Launch { query, claude_args } => {
            let cfg = load_config()?;
            let account = match_account(&cfg.accounts, &query).map_err(|e| {
                let reason = match e {
                    AccountMatchError::NoMatch => format!("no account matches '{query}'"),
                    AccountMatchError::Ambiguous(ids) => {
                        format!("'{query}' is ambiguous ({})", ids.join(", "))
                    }
                };
                format!("{reason}. Available accounts:\n{}", account_list(&cfg))
            })?;

            session::ensure_session_usable(account)?;

            // Same inherit pass as the tray launch; the CLI never opens a GUI
            // prompt and never persists decisions — undecided conflicts are
            // skipped for this launch and resolved from the tray.
            let source = paths::expand_tilde("~/.claude");
            if source.is_dir() {
                let config_dir = paths::expand_tilde(&account.config_dir);
                match inherit::seed_and_apply(&source, &config_dir, &account.inherit_overrides) {
                    Ok(outcome) => {
                        if let Some(e) = outcome.seed_error {
                            eprintln!("warning: settings.json seed failed: {e}");
                        }
                        for sub in &outcome.needs_prompt {
                            eprintln!(
                                "warning: '{sub}' has a conflict with ~/.claude and no saved decision; launching without inheriting it. Launch once from the tray to resolve."
                            );
                        }
                    }
                    Err(e) => {
                        if let Some(s) = e.seed_error {
                            eprintln!("warning: settings.json seed failed: {s}");
                        }
                        return Err(format!("inheriting ~/.claude failed: {}", e.inherit_error));
                    }
                }
            }

            exec_claude(account, &claude_args)
        }
    }
}

#[cfg(unix)]
fn exec_claude(
    account: &claude_multi_lib::config::Account,
    claude_args: &[String],
) -> Result<ExitCode, String> {
    use std::os::unix::process::CommandExt;
    let mut cmd = std::process::Command::new("claude");
    cmd.args(claude_args);
    for (k, v) in launch_plan(account).env {
        cmd.env(k, v);
    }
    // exec only returns on failure.
    Err(format!("failed to launch claude: {}", cmd.exec()))
}

#[cfg(not(unix))]
fn exec_claude(
    account: &claude_multi_lib::config::Account,
    claude_args: &[String],
) -> Result<ExitCode, String> {
    let mut cmd = std::process::Command::new("claude");
    cmd.args(claude_args);
    for (k, v) in launch_plan(account).env {
        cmd.env(k, v);
    }
    let status = cmd
        .status()
        .map_err(|e| format!("failed to launch claude: {e}"))?;
    Ok(ExitCode::from(
        u8::try_from(status.code().unwrap_or(1)).unwrap_or(1),
    ))
}

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(msg) => {
            eprintln!("cms: {msg}");
            ExitCode::FAILURE
        }
    }
}
