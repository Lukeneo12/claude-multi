//! Detects whether an account's Claude Code login is still usable.
//!
//! `.claude.json` keeps `oauthAccount` (the email) until an explicit logout, so
//! it can't tell an expired session from a live one. The OAuth tokens can: they
//! live in `<config_dir>/.credentials.json` or, on macOS, in the login Keychain
//! under the service `Claude Code-credentials-<hash>` where `<hash>` is the
//! first 8 hex chars of `sha256(CLAUDE_CONFIG_DIR)` — the exact string the
//! launch scripts export, so hashing the expanded configured path matches.
//!
//! Verdicts are conservative: only positive evidence of expiry (a past
//! `refreshTokenExpiresAt`, or a past `expiresAt` with no refresh token) marks
//! an account `Expired`. Unreadable or missing credentials keep the account
//! `LoggedIn` so a denied Keychain prompt never locks the user out.

use crate::config::Account;
use crate::paths::expand_tilde;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionStatus {
    LoggedOut,
    LoggedIn { email: String },
    Expired { email: String },
}

/// What the stored OAuth credentials say about the session, independent of
/// whether the account has an email recorded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CredentialVerdict {
    /// Tokens present and not past their expiry.
    Valid,
    /// Positive evidence the session can no longer refresh itself.
    Expired,
    /// Missing/unreadable/unparseable credentials — assume nothing.
    Unknown,
}

/// Keychain service name Claude Code uses for a non-default `CLAUDE_CONFIG_DIR`.
/// macOS-only at runtime (other OSes keep `.credentials.json`), but the hash is
/// pure and unit-tested on every OS.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn keychain_service(config_dir: &str) -> String {
    let digest = Sha256::digest(config_dir.as_bytes());
    let hash8: String = digest.iter().take(4).map(|b| format!("{b:02x}")).collect();
    format!("Claude Code-credentials-{hash8}")
}

/// Pure expiry evaluation over the credentials JSON (`claudeAiOauth` payload).
/// `now_ms` is Unix epoch milliseconds, matching the stored timestamps.
pub fn evaluate_credentials(json: &str, now_ms: i64) -> CredentialVerdict {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(json) else {
        return CredentialVerdict::Unknown;
    };
    let Some(oauth) = value.get("claudeAiOauth") else {
        return CredentialVerdict::Unknown;
    };

    // The refresh token is what keeps a session alive across access-token
    // expiries; its own expiry is the real "session lost" signal.
    if let Some(refresh_exp) = oauth.get("refreshTokenExpiresAt").and_then(|v| v.as_i64()) {
        return if refresh_exp <= now_ms {
            CredentialVerdict::Expired
        } else {
            CredentialVerdict::Valid
        };
    }

    let has_refresh_token = oauth
        .get("refreshToken")
        .and_then(|v| v.as_str())
        .is_some_and(|s| !s.is_empty());
    if has_refresh_token {
        return CredentialVerdict::Valid; // refreshable, expiry unknown
    }

    match oauth.get("expiresAt").and_then(|v| v.as_i64()) {
        Some(exp) if exp <= now_ms => CredentialVerdict::Expired,
        Some(_) => CredentialVerdict::Valid,
        None => CredentialVerdict::Unknown,
    }
}

/// Reads the raw credentials JSON for one config dir: the `.credentials.json`
/// file when present (Linux/Windows and older CLIs), else the macOS Keychain.
fn read_credentials(config_dir: &Path) -> Option<String> {
    let file = config_dir.join(".credentials.json");
    if let Ok(contents) = std::fs::read_to_string(&file) {
        return Some(contents);
    }
    read_keychain_credentials(config_dir)
}

#[cfg(target_os = "macos")]
fn read_keychain_credentials(config_dir: &Path) -> Option<String> {
    let service = keychain_service(&config_dir.to_string_lossy());
    let output = std::process::Command::new("security")
        .args(["find-generic-password", "-w", "-s", &service])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout).ok()
}

#[cfg(not(target_os = "macos"))]
fn read_keychain_credentials(_config_dir: &Path) -> Option<String> {
    None
}

/// How long a credential verdict is reused before re-reading its source.
/// Expiries move on a scale of days, while the tray rebuilds on every hover —
/// the cache keeps that rebuild from shelling out to `security` (and from
/// re-showing a denied Keychain prompt) on each open. It also amortizes the
/// double read per launch (menu render + launch gate).
const VERDICT_TTL: Duration = Duration::from_secs(30);

fn verdict_cache() -> &'static Mutex<HashMap<PathBuf, (Instant, CredentialVerdict)>> {
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, (Instant, CredentialVerdict)>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Cached wrapper around the credential read + evaluation for one config dir.
fn cached_verdict(config_dir: &Path) -> CredentialVerdict {
    if let Ok(cache) = verdict_cache().lock() {
        if let Some((read_at, verdict)) = cache.get(config_dir) {
            if read_at.elapsed() < VERDICT_TTL {
                return *verdict;
            }
        }
    }
    let verdict = match read_credentials(config_dir) {
        Some(json) => evaluate_credentials(&json, chrono::Utc::now().timestamp_millis()),
        None => CredentialVerdict::Unknown,
    };
    if let Ok(mut cache) = verdict_cache().lock() {
        cache.insert(config_dir.to_path_buf(), (Instant::now(), verdict));
    }
    verdict
}

/// Full status for one account, combining the recorded email with the token
/// verdict. Never blocks a user out on missing evidence (see module docs).
pub fn account_session_status(account: &Account) -> SessionStatus {
    let Some(email) = account.logged_in_email() else {
        return SessionStatus::LoggedOut;
    };
    let config_dir = expand_tilde(&account.config_dir);
    match cached_verdict(&config_dir) {
        CredentialVerdict::Expired => SessionStatus::Expired { email },
        CredentialVerdict::Valid | CredentialVerdict::Unknown => SessionStatus::LoggedIn { email },
    }
}

/// Launch-time gate: sessions and project launches refuse an expired account
/// instead of opening a terminal that will just ask for login.
pub fn ensure_session_usable(account: &Account) -> Result<(), String> {
    match account_session_status(account) {
        SessionStatus::Expired { email } => Err(format!(
            "The session for '{}' ({email}) has expired. Use “Re-login…” in this account's tray menu first.",
            account.label
        )),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Account, UsageLimits};
    use std::collections::HashMap;

    const NOW: i64 = 1_785_000_000_000; // fixed "now" in epoch ms

    fn account_with_dir(dir: &Path) -> Account {
        Account {
            id: "x".into(),
            label: "X".into(),
            config_dir: dir.to_string_lossy().to_string(),
            inherit_overrides: HashMap::new(),
            usage_limits: UsageLimits::default(),
        }
    }

    #[test]
    fn test_should_derive_keychain_service_from_config_dir_hash() {
        // Locks the algorithm (first 8 hex chars of sha256 over the raw path),
        // which was verified once against real Claude Code Keychain entries.
        assert_eq!(
            keychain_service("/Users/jdoe/.claude-personal"),
            "Claude Code-credentials-2b0fd335"
        );
    }

    #[test]
    fn test_should_report_expired_when_refresh_token_past_expiry() {
        let json = format!(
            r#"{{"claudeAiOauth":{{"refreshTokenExpiresAt":{}}}}}"#,
            NOW - 1
        );
        assert_eq!(evaluate_credentials(&json, NOW), CredentialVerdict::Expired);
    }

    #[test]
    fn test_should_report_valid_when_refresh_token_still_live() {
        let json = format!(
            r#"{{"claudeAiOauth":{{"expiresAt":{},"refreshTokenExpiresAt":{}}}}}"#,
            NOW - 1, // access token expired is fine — it refreshes
            NOW + 86_400_000
        );
        assert_eq!(evaluate_credentials(&json, NOW), CredentialVerdict::Valid);
    }

    #[test]
    fn test_should_report_valid_when_refresh_token_present_without_expiry() {
        let json = format!(
            r#"{{"claudeAiOauth":{{"expiresAt":{},"refreshToken":"tok"}}}}"#,
            NOW - 1
        );
        assert_eq!(evaluate_credentials(&json, NOW), CredentialVerdict::Valid);
    }

    #[test]
    fn test_should_report_expired_when_access_token_past_and_no_refresh_token() {
        let json = format!(r#"{{"claudeAiOauth":{{"expiresAt":{}}}}}"#, NOW - 1);
        assert_eq!(evaluate_credentials(&json, NOW), CredentialVerdict::Expired);
    }

    #[test]
    fn test_should_report_unknown_when_json_malformed_or_missing_oauth() {
        assert_eq!(
            evaluate_credentials("not json", NOW),
            CredentialVerdict::Unknown
        );
        assert_eq!(evaluate_credentials("{}", NOW), CredentialVerdict::Unknown);
    }

    #[test]
    fn test_should_report_logged_out_when_no_email_recorded() {
        let account = account_with_dir(Path::new("/nonexistent/cm-session-dir"));
        assert_eq!(account_session_status(&account), SessionStatus::LoggedOut);
    }

    /// Unique-per-run account dir (RAII cleanup even on assert panic), seeded
    /// as logged-in with the given email.
    fn logged_in_dir(email: &str) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join(".claude.json"),
            format!(r#"{{"oauthAccount":{{"emailAddress":"{email}"}}}}"#),
        )
        .unwrap();
        dir
    }

    const EXPIRED_CREDS: &str = r#"{"claudeAiOauth":{"refreshTokenExpiresAt":1}}"#;

    #[test]
    fn test_should_report_expired_when_credentials_file_past_refresh_expiry() {
        let dir = logged_in_dir("a@b.c");
        std::fs::write(dir.path().join(".credentials.json"), EXPIRED_CREDS).unwrap();
        let status = account_session_status(&account_with_dir(dir.path()));
        assert_eq!(
            status,
            SessionStatus::Expired {
                email: "a@b.c".into()
            }
        );
    }

    #[test]
    fn test_should_allow_launch_when_account_logged_out() {
        // The gate only blocks *expired* sessions; other states pass through
        // (the tray already hides launch items for logged-out accounts).
        let account = account_with_dir(Path::new("/nonexistent/cm-gate-dir"));
        assert!(ensure_session_usable(&account).is_ok());
    }

    #[test]
    fn test_should_stay_logged_in_when_credentials_unreadable() {
        // Email present but no credentials source at all → conservative LoggedIn.
        let dir = logged_in_dir("a@b.c");
        let status = account_session_status(&account_with_dir(dir.path()));
        // On macOS this may consult the Keychain for a temp-dir service that
        // can't exist, which cleanly reports "not found" → Unknown → LoggedIn.
        assert_eq!(
            status,
            SessionStatus::LoggedIn {
                email: "a@b.c".into()
            }
        );
    }

    #[test]
    fn test_should_refuse_launch_when_session_expired() {
        let dir = logged_in_dir("a@b.c");
        std::fs::write(dir.path().join(".credentials.json"), EXPIRED_CREDS).unwrap();
        let err = ensure_session_usable(&account_with_dir(dir.path())).unwrap_err();
        assert!(err.contains("expired"));
        assert!(err.contains("Re-login"));
    }

    #[test]
    fn test_should_reuse_cached_verdict_when_credentials_change_within_ttl() {
        // The verdict is cached per config dir for VERDICT_TTL, so the tray's
        // hover-rebuild doesn't re-read credentials (or re-shell to `security`)
        // on every open. Freshly written valid credentials therefore aren't
        // seen until the TTL lapses — an accepted staleness of seconds against
        // expiries measured in days.
        let dir = logged_in_dir("a@b.c");
        std::fs::write(dir.path().join(".credentials.json"), EXPIRED_CREDS).unwrap();
        let account = account_with_dir(dir.path());
        assert!(matches!(
            account_session_status(&account),
            SessionStatus::Expired { .. }
        ));

        // Far-future expiry (year 2100) so the test can only pass via the
        // cache — a fresh read would evaluate these credentials as Valid.
        let valid = r#"{"claudeAiOauth":{"refreshTokenExpiresAt":4102444800000}}"#;
        std::fs::write(dir.path().join(".credentials.json"), valid).unwrap();
        assert!(
            matches!(
                account_session_status(&account),
                SessionStatus::Expired { .. }
            ),
            "verdict must come from the cache within the TTL"
        );
    }
}
