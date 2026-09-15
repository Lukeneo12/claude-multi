//! Pure decision logic for the `cms` CLI bin: arg parsing, account matching,
//! and launch-plan assembly. No I/O — the bin (`src/bin/cms.rs`) owns all
//! printing, process spawning, and filesystem access.

use crate::config::Account;

/// What the `cms` invocation asks for. `Help { explicit }` distinguishes
/// `cms --help` (exit 0) from `cms` with no args (usage error, exit != 0).
#[derive(Debug, PartialEq)]
pub enum CliCommand {
    Help { explicit: bool },
    List,
    Launch {
        query: String,
        claude_args: Vec<String>,
    },
}

/// Why an account query failed to resolve. `Ambiguous` carries the candidate
/// ids so the caller can list them.
#[derive(Debug, PartialEq)]
pub enum AccountMatchError {
    NoMatch,
    Ambiguous(Vec<String>),
}

/// Resolves a user-typed query against configured accounts, case-insensitively
/// over both `id` and `label`. A unique exact match wins outright (an exact
/// hit on one account's `id` and another's `label` is ambiguous); otherwise a
/// unique prefix match resolves, and multiple prefix candidates are an error.
pub fn match_account<'a>(
    accounts: &'a [Account],
    query: &str,
) -> Result<&'a Account, AccountMatchError> {
    let q = query.to_lowercase();
    if q.is_empty() {
        return Err(AccountMatchError::NoMatch);
    }
    let exact: Vec<&Account> = accounts
        .iter()
        .filter(|a| a.id.to_lowercase() == q || a.label.to_lowercase() == q)
        .collect();
    match exact.len() {
        0 => {}
        1 => return Ok(exact[0]),
        _ => {
            return Err(AccountMatchError::Ambiguous(
                exact.iter().map(|a| a.id.clone()).collect(),
            ))
        }
    }
    let candidates: Vec<&Account> = accounts
        .iter()
        .filter(|a| {
            a.id.to_lowercase().starts_with(&q) || a.label.to_lowercase().starts_with(&q)
        })
        .collect();
    match candidates.len() {
        0 => Err(AccountMatchError::NoMatch),
        1 => Ok(candidates[0]),
        _ => Err(AccountMatchError::Ambiguous(
            candidates.iter().map(|a| a.id.clone()).collect(),
        )),
    }
}

/// Parses `cms` args (without the program name). The first arg selects the
/// command; everything after an account query passes through to `claude`
/// verbatim, in order. `--list`/`--help` take no extra args, and an unknown
/// leading flag is a usage error rather than an account query.
pub fn parse_cli_args(args: &[String]) -> CliCommand {
    match args.first().map(String::as_str) {
        None => CliCommand::Help { explicit: false },
        Some("--help") | Some("-h") if args.len() == 1 => CliCommand::Help { explicit: true },
        Some("--list") | Some("-l") if args.len() == 1 => CliCommand::List,
        Some(flag) if flag.starts_with('-') => CliCommand::Help { explicit: false },
        Some(query) => CliCommand::Launch {
            query: query.to_string(),
            claude_args: args[1..].to_vec(),
        },
    }
}

/// Environment the `cms` bin sets on the `claude` child process. Values are
/// native paths (via `PathBuf::join`) — no shell text is generated, so the
/// script-escaping invariant does not apply here.
pub struct LaunchPlan {
    pub env: Vec<(String, String)>,
}

/// Builds the env for launching `claude` under `account`:
/// `CLAUDE_CONFIG_DIR` first, then one entry per
/// `launcher::PER_ACCOUNT_ENV_VARS` (`<config_dir>/<subdir>`), mirroring the
/// tray's script builders.
pub fn launch_plan(account: &Account) -> LaunchPlan {
    let dir = crate::paths::expand_tilde(&account.config_dir);
    let mut env = vec![(
        "CLAUDE_CONFIG_DIR".to_string(),
        dir.to_string_lossy().into_owned(),
    )];
    for (var, subdir) in crate::launcher::PER_ACCOUNT_ENV_VARS {
        env.push((
            (*var).to_string(),
            dir.join(subdir).to_string_lossy().into_owned(),
        ));
    }
    LaunchPlan { env }
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
    fn test_should_return_usage_error_when_list_has_extra_args() {
        assert_eq!(
            parse_cli_args(&args(&["--list", "foo"])),
            CliCommand::Help { explicit: false }
        );
        assert_eq!(
            parse_cli_args(&args(&["-l", "foo"])),
            CliCommand::Help { explicit: false }
        );
    }

    #[test]
    fn test_should_return_usage_error_when_help_has_extra_args() {
        assert_eq!(
            parse_cli_args(&args(&["--help", "foo"])),
            CliCommand::Help { explicit: false }
        );
    }

    #[test]
    fn test_should_return_usage_error_when_first_arg_is_unknown_flag() {
        assert_eq!(
            parse_cli_args(&args(&["--bogus"])),
            CliCommand::Help { explicit: false }
        );
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

    use crate::config::{Account, UsageLimits};
    use std::collections::HashMap;

    fn account(id: &str, label: &str) -> Account {
        Account {
            id: id.to_string(),
            label: label.to_string(),
            config_dir: format!("~/.claude-{id}"),
            inherit_overrides: HashMap::new(),
            usage_limits: UsageLimits::default(),
        }
    }

    fn fixture_accounts() -> Vec<Account> {
        vec![account("personal", "Personal"), account("a1", "Dinocloud")]
    }

    #[test]
    fn test_should_match_by_exact_id_when_query_is_id() {
        let accounts = fixture_accounts();
        assert_eq!(match_account(&accounts, "a1").unwrap().id, "a1");
    }

    #[test]
    fn test_should_match_case_insensitively_when_query_is_label() {
        let accounts = fixture_accounts();
        assert_eq!(match_account(&accounts, "DINOCLOUD").unwrap().id, "a1");
    }

    #[test]
    fn test_should_match_by_unique_prefix_when_query_shortened() {
        let accounts = fixture_accounts();
        assert_eq!(match_account(&accounts, "dino").unwrap().id, "a1");
        assert_eq!(match_account(&accounts, "per").unwrap().id, "personal");
    }

    #[test]
    fn test_should_prefer_exact_match_when_query_is_also_prefix_of_other() {
        // "work" is exact for one account and a prefix of "workshop".
        let accounts = vec![account("work", "Work"), account("workshop", "Workshop")];
        assert_eq!(match_account(&accounts, "work").unwrap().id, "work");
    }

    #[test]
    fn test_should_error_no_match_when_query_matches_nothing() {
        let accounts = fixture_accounts();
        assert_eq!(
            match_account(&accounts, "x").unwrap_err(),
            AccountMatchError::NoMatch
        );
    }

    #[test]
    fn test_should_error_no_match_when_query_empty() {
        let accounts = fixture_accounts();
        assert_eq!(
            match_account(&accounts, "").unwrap_err(),
            AccountMatchError::NoMatch
        );
    }

    #[test]
    fn test_should_error_ambiguous_when_query_is_exact_id_of_one_and_exact_label_of_another() {
        let accounts = vec![account("work", "Job"), account("a2", "Work")];
        assert_eq!(
            match_account(&accounts, "work").unwrap_err(),
            AccountMatchError::Ambiguous(vec!["work".to_string(), "a2".to_string()])
        );
    }

    #[test]
    fn test_should_match_when_query_is_both_id_and_label_of_same_account() {
        let accounts = vec![account("work", "Work"), account("a2", "Other")];
        assert_eq!(match_account(&accounts, "work").unwrap().id, "work");
    }

    #[test]
    fn test_should_error_ambiguous_with_candidate_ids_when_prefix_matches_many() {
        let accounts = vec![account("dev1", "Dev One"), account("dev2", "Dev Two")];
        assert_eq!(
            match_account(&accounts, "dev").unwrap_err(),
            AccountMatchError::Ambiguous(vec!["dev1".to_string(), "dev2".to_string()])
        );
    }

    #[test]
    fn test_should_expand_tilde_when_building_launch_plan() {
        let plan = launch_plan(&account("personal", "Personal"));
        let home = std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .unwrap();
        let expected_dir = std::path::PathBuf::from(&home).join(".claude-personal");
        assert_eq!(
            plan.env[0],
            (
                "CLAUDE_CONFIG_DIR".to_string(),
                expected_dir.to_string_lossy().into_owned()
            )
        );
    }

    #[test]
    fn test_should_include_per_account_env_vars_when_building_launch_plan() {
        let plan = launch_plan(&account("personal", "Personal"));
        let gh = plan
            .env
            .iter()
            .find(|(k, _)| k == "GH_CONFIG_DIR")
            .expect("GH_CONFIG_DIR present");
        let claude_dir = &plan.env[0].1;
        assert_eq!(
            gh.1,
            std::path::Path::new(claude_dir)
                .join("gh")
                .to_string_lossy()
                .into_owned()
        );
    }
}
