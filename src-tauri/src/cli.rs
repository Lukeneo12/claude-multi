//! Pure decision logic for the `cms` CLI bin: arg parsing, account matching,
//! and launch-plan assembly. No I/O — the bin (`src/bin/cms.rs`) owns all
//! printing, process spawning, and filesystem access.

/// What the `cms` invocation asks for. `Help { explicit }` distinguishes
/// `cms --help` (exit 0) from `cms` with no args (usage error, exit != 0).
#[derive(Debug, PartialEq)]
#[allow(dead_code)]
pub enum CliCommand {
    Help { explicit: bool },
    List,
    Launch {
        query: String,
        claude_args: Vec<String>,
    },
}

/// Parses `cms` args (without the program name). The first arg selects the
/// command; everything after an account query passes through to `claude`
/// verbatim, in order.
#[allow(dead_code)]
pub fn parse_cli_args(args: &[String]) -> CliCommand {
    match args.first().map(String::as_str) {
        None => CliCommand::Help { explicit: false },
        Some("--help") | Some("-h") => CliCommand::Help { explicit: true },
        Some("--list") | Some("-l") => CliCommand::List,
        Some(query) => CliCommand::Launch {
            query: query.to_string(),
            claude_args: args[1..].to_vec(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn test_should_return_implicit_help_when_no_args() {
        assert_eq!(parse_cli_args(&[]), CliCommand::Help { explicit: false });
    }

    #[test]
    fn test_should_return_explicit_help_when_help_flag() {
        assert_eq!(
            parse_cli_args(&args(&["--help"])),
            CliCommand::Help { explicit: true }
        );
        assert_eq!(
            parse_cli_args(&args(&["-h"])),
            CliCommand::Help { explicit: true }
        );
    }

    #[test]
    fn test_should_return_list_when_list_flag() {
        assert_eq!(parse_cli_args(&args(&["--list"])), CliCommand::List);
        assert_eq!(parse_cli_args(&args(&["-l"])), CliCommand::List);
    }

    #[test]
    fn test_should_pass_trailing_args_through_when_launching() {
        assert_eq!(
            parse_cli_args(&args(&["dino", "--resume", "-p", "x y"])),
            CliCommand::Launch {
                query: "dino".to_string(),
                claude_args: args(&["--resume", "-p", "x y"]),
            }
        );
    }

    #[test]
    fn test_should_launch_with_empty_claude_args_when_only_account_given() {
        assert_eq!(
            parse_cli_args(&args(&["personal"])),
            CliCommand::Launch {
                query: "personal".to_string(),
                claude_args: vec![],
            }
        );
    }
}
