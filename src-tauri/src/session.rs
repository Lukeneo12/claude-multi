//! Detects whether an account's Claude Code login is still usable.
//!
//! `.claude.json` keeps `oauthAccount` (the email) until an explicit logout, so
//! it can't tell an expired session from a live one. The OAuth tokens can: they
//! live in `<config_dir>/.credentials.json` or, on macOS, in the login Keychain
//! under the service `Claude Code-credentials-<hash>` where `<hash>` is the
//! first 8 hex chars of `sha256(CLAUDE_CONFIG_DIR)` — the exact string the
//! launch scripts export, so hashing the expanded configured path matches.
//!
//! Verdicts are conservative about *expiry*: only positive evidence (a past
//! `refreshTokenExpiresAt`, or a past `expiresAt` with no refresh token) marks
//! an account `Expired`, and credentials that exist but can't be read (e.g. a
//! denied Keychain prompt) keep the account `LoggedIn` so the user is never
//! locked out. Credentials that are positively *absent* (no file, Keychain item
//! not found) are a different matter: that is exactly what a logout leaves
//! behind — `.claude.json` may still record the email — so the account is
//! `LoggedOut`. This mirrors the CLI itself, whose `claude auth status` reports
//! `loggedIn: false` from the credentials alone.

use crate::config::Account;
use crate::paths::expand_tilde;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionStatus {
    /// Not usable. `last_email` is the email `.claude.json` still records when
    /// the tokens are gone (so the tray can say *which* account logged out);
    /// `None` for an account that never logged in.
    LoggedOut {
        last_email: Option<String>,
    },
    LoggedIn {
        email: String,
    },
    Expired {
        email: String,
    },
}

/// What the stored OAuth credentials say about the session, independent of
/// whether the account has an email recorded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CredentialVerdict {
    /// Tokens present and not past their expiry.
    Valid,
    /// Positive evidence the session can no longer refresh itself.
    Expired,
    /// Credentials exist but are unreadable/unparseable — assume nothing.
    Unknown,
    /// No credentials at all (no file, Keychain item not found): logged out.
    Absent,
}

/// Raw outcome of looking for the credentials of one config dir.
#[derive(Debug, Clone, PartialEq, Eq)]
enum CredentialSource {
    Found(String),
    Absent,
    Unreadable,
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

/// Looks up the raw credentials JSON for one config dir: the
/// `.credentials.json` file when present (Linux/Windows and older CLIs), else
/// the macOS Keychain. Distinguishes "nothing there" from "there but unreadable".
fn read_credentials(config_dir: &Path) -> CredentialSource {
    let file = config_dir.join(".credentials.json");
    match std::fs::read_to_string(&file) {
        Ok(contents) => CredentialSource::Found(contents),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => read_keychain_credentials(config_dir),
        Err(_) => CredentialSource::Unreadable,
    }
}

/// True when a failed `security find-generic-password` run means the item does
/// not exist (exit status 44 / "The specified item could not be found"), as
/// opposed to existing but being unreadable (denied prompt, interaction not
/// allowed, …). Pure so it's testable on every OS; matching both signals keeps
/// it robust to either one changing.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn security_item_not_found(exit_code: Option<i32>, stderr: &str) -> bool {
    exit_code == Some(44) || stderr.contains("could not be found")
}

#[cfg(target_os = "macos")]
fn read_keychain_credentials(config_dir: &Path) -> CredentialSource {
    let service = keychain_service(&config_dir.to_string_lossy());
    let Ok(output) = std::process::Command::new("security")
        .args(["find-generic-password", "-w", "-s", &service])
        .output()
    else {
        return CredentialSource::Unreadable;
    };
    if output.status.success() {
        return match String::from_utf8(output.stdout) {
            Ok(json) => CredentialSource::Found(json),
            Err(_) => CredentialSource::Unreadable,
        };
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    if security_item_not_found(output.status.code(), &stderr) {
        CredentialSource::Absent
    } else {
        CredentialSource::Unreadable
    }
}

#[cfg(not(target_os = "macos"))]
fn read_keychain_credentials(_config_dir: &Path) -> CredentialSource {
    // No Keychain elsewhere: a missing `.credentials.json` is the whole story.
    CredentialSource::Absent
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

/// After a tray auth action (login / logout / re-login) the credentials change
/// *later*, once the user finishes in the terminal — so for this long the dir
/// is read fresh on every menu build instead of re-caching the pre-action
/// state on the first hover. Reads without a prompt are cheap; the window only
/// affects the one account being acted on.
const AUTH_GRACE: Duration = Duration::from_secs(120);

fn no_cache_until() -> &'static Mutex<HashMap<PathBuf, Instant>> {
    static GRACE: OnceLock<Mutex<HashMap<PathBuf, Instant>>> = OnceLock::new();
    GRACE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn in_grace(config_dir: &Path) -> bool {
    no_cache_until()
        .lock()
        .ok()
        .and_then(|g| g.get(config_dir).copied())
        .is_some_and(|until| Instant::now() < until)
}

/// Drops the cached verdict for one config dir and opens an [`AUTH_GRACE`]
/// window during which it is not cached again. Called when a tray action is
/// about to change the credentials, so the menu tracks the terminal's outcome
/// as it happens rather than serving a stale verdict for the rest of the TTL.
pub fn invalidate_verdict(config_dir: &Path) {
    if let Ok(mut cache) = verdict_cache().lock() {
        cache.remove(config_dir);
    }
    if let Ok(mut grace) = no_cache_until().lock() {
        grace.insert(config_dir.to_path_buf(), Instant::now() + AUTH_GRACE);
    }
}

/// Cached wrapper around the credential read + evaluation for one config dir.
/// `Absent` is never cached: a missing item returns instantly and never
/// prompts, and not caching it means a fresh login shows up on the next hover
/// instead of after the TTL. Nothing is cached inside an auth grace window.
fn cached_verdict(config_dir: &Path) -> CredentialVerdict {
    let in_grace = in_grace(config_dir);
    if !in_grace {
        if let Ok(cache) = verdict_cache().lock() {
            if let Some((read_at, verdict)) = cache.get(config_dir) {
                if read_at.elapsed() < VERDICT_TTL {
                    return *verdict;
                }
            }
        }
    }
    let verdict = match read_credentials(config_dir) {
        CredentialSource::Found(json) => {
            evaluate_credentials(&json, chrono::Utc::now().timestamp_millis())
        }
        CredentialSource::Unreadable => CredentialVerdict::Unknown,
        CredentialSource::Absent => CredentialVerdict::Absent,
    };
    if !in_grace && verdict != CredentialVerdict::Absent {
        if let Ok(mut cache) = verdict_cache().lock() {
            cache.insert(config_dir.to_path_buf(), (Instant::now(), verdict));
        }
    }
    verdict
}

/// Pure mapping from the recorded email + credential verdict to a status.
pub fn status_from(email: String, verdict: CredentialVerdict) -> SessionStatus {
    match verdict {
        CredentialVerdict::Absent => SessionStatus::LoggedOut {
            last_email: Some(email),
        },
        CredentialVerdict::Expired => SessionStatus::Expired { email },
        CredentialVerdict::Valid | CredentialVerdict::Unknown => SessionStatus::LoggedIn { email },
    }
}

/// Full status for one account, combining the recorded email with the token
/// verdict. Never locks a user out on *unreadable* evidence (see module docs);
/// only positively absent credentials read as logged out.
pub fn account_session_status(account: &Account) -> SessionStatus {
    let Some(email) = account.logged_in_email() else {
        return SessionStatus::LoggedOut { last_email: None }; // nothing to read
    };
    status_from(email, cached_verdict(&expand_tilde(&account.config_dir)))
}

/// Launch-time gate: sessions and project launches refuse a logged-out or
/// expired account instead of opening a terminal that will just ask for login.
pub fn ensure_session_usable(account: &Account) -> Result<(), String> {
    match account_session_status(account) {
        SessionStatus::LoggedOut { .. } => Err(format!(
            "'{}' is not logged in. Use “Login…” in this account's tray menu first.",
            account.label
        )),
        SessionStatus::Expired { email } => Err(format!(
            "The session for '{}' ({email}) has expired. Use “Re-login…” in this account's tray menu first.",
            account.label
        )),
        SessionStatus::LoggedIn { .. } => Ok(()),
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
        assert_eq!(
            account_session_status(&account),
            SessionStatus::LoggedOut { last_email: None }
        );
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
    fn test_should_refuse_launch_when_account_never_logged_in() {
        // No email recorded → LoggedOut → the gate refuses and points at Login…
        // (backstop for a stale menu or a direct `invoke`).
        let account = account_with_dir(Path::new("/nonexistent/cm-gate-dir"));
        let err = ensure_session_usable(&account).unwrap_err();
        assert!(err.contains("not logged in"));
        assert!(err.contains("Login"));
    }

    #[test]
    fn test_should_report_logged_out_when_email_recorded_but_credentials_absent() {
        // The logout leftover: `.claude.json` still names the account, but the
        // tokens are gone (no file; on macOS the Keychain item for a temp-dir
        // service can't exist → "not found"). That is a logged-out account.
        let dir = logged_in_dir("a@b.c");
        let status = account_session_status(&account_with_dir(dir.path()));
        assert_eq!(
            status,
            SessionStatus::LoggedOut {
                last_email: Some("a@b.c".into())
            }
        );
    }

    #[test]
    fn test_should_refuse_launch_when_email_recorded_but_credentials_absent() {
        let dir = logged_in_dir("a@b.c");
        let err = ensure_session_usable(&account_with_dir(dir.path())).unwrap_err();
        assert!(err.contains("not logged in"));
        assert!(err.contains("Login"));
    }

    #[test]
    fn test_should_stay_logged_in_when_credentials_unreadable() {
        // Email present and a credentials source that exists but can't be read
        // (a directory where the file should be) → Unknown → conservative
        // LoggedIn, never a lock-out.
        let dir = logged_in_dir("a@b.c");
        std::fs::create_dir(dir.path().join(".credentials.json")).unwrap();
        let status = account_session_status(&account_with_dir(dir.path()));
        assert_eq!(
            status,
            SessionStatus::LoggedIn {
                email: "a@b.c".into()
            }
        );
    }

    #[test]
    fn test_should_detect_security_item_not_found_from_exit_code_or_message() {
        assert!(security_item_not_found(
            Some(44),
            "security: SecKeychainSearchCopyNext: The specified item could not be found.\n"
        ));
        // Either signal alone is enough.
        assert!(security_item_not_found(Some(44), ""));
        assert!(security_item_not_found(
            Some(1),
            "The specified item could not be found."
        ));
    }

    #[test]
    fn test_should_not_treat_other_security_failures_as_not_found() {
        assert!(!security_item_not_found(
            Some(128),
            "security: SecKeychainItemCopyContent: User canceled."
        ));
        assert!(!security_item_not_found(
            Some(36),
            "User interaction is not allowed."
        ));
        assert!(!security_item_not_found(None, ""));
    }

    #[test]
    fn test_should_map_verdicts_to_status_when_email_recorded() {
        let email = || "a@b.c".to_string();
        assert_eq!(
            status_from(email(), CredentialVerdict::Valid),
            SessionStatus::LoggedIn { email: email() }
        );
        assert_eq!(
            status_from(email(), CredentialVerdict::Unknown),
            SessionStatus::LoggedIn { email: email() }
        );
        assert_eq!(
            status_from(email(), CredentialVerdict::Expired),
            SessionStatus::Expired { email: email() }
        );
        assert_eq!(
            status_from(email(), CredentialVerdict::Absent),
            SessionStatus::LoggedOut {
                last_email: Some(email())
            }
        );
    }

    #[test]
    fn test_should_not_cache_absent_verdict_when_credentials_appear_later() {
        // Absent is never cached: a login right after a hover must show up on
        // the next menu build, not after the TTL.
        let dir = logged_in_dir("a@b.c");
        let account = account_with_dir(dir.path());
        assert!(matches!(
            account_session_status(&account),
            SessionStatus::LoggedOut { .. }
        ));
        let valid = r#"{"claudeAiOauth":{"refreshTokenExpiresAt":4102444800000}}"#;
        std::fs::write(dir.path().join(".credentials.json"), valid).unwrap();
        assert_eq!(
            account_session_status(&account),
            SessionStatus::LoggedIn {
                email: "a@b.c".into()
            }
        );
    }

    #[test]
    fn test_should_reread_credentials_when_verdict_invalidated() {
        let dir = logged_in_dir("a@b.c");
        std::fs::write(dir.path().join(".credentials.json"), EXPIRED_CREDS).unwrap();
        let account = account_with_dir(dir.path());
        assert!(matches!(
            account_session_status(&account),
            SessionStatus::Expired { .. }
        ));
        // Simulate a logout: tokens gone. The cache would still say Expired…
        std::fs::remove_file(dir.path().join(".credentials.json")).unwrap();
        assert!(matches!(
            account_session_status(&account),
            SessionStatus::Expired { .. }
        ));
        // …until the tray action drops it.
        invalidate_verdict(dir.path());
        assert!(matches!(
            account_session_status(&account),
            SessionStatus::LoggedOut { .. }
        ));
    }

    #[test]
    fn test_should_keep_reading_fresh_during_auth_grace_after_invalidation() {
        // A hover *during* the terminal action must not re-cache the pre-action
        // state: after invalidate_verdict the dir stays uncached for AUTH_GRACE,
        // so the state that lands when the action completes shows immediately.
        let dir = logged_in_dir("a@b.c");
        std::fs::write(dir.path().join(".credentials.json"), EXPIRED_CREDS).unwrap();
        let account = account_with_dir(dir.path());
        invalidate_verdict(dir.path()); // "Re-login…" clicked
                                        // Hover while the terminal is still open: reads Expired (would cache it
                                        // for VERDICT_TTL without the grace window).
        assert!(matches!(
            account_session_status(&account),
            SessionStatus::Expired { .. }
        ));
        // The re-login lands; the next hover must already see it.
        let valid = r#"{"claudeAiOauth":{"refreshTokenExpiresAt":4102444800000}}"#;
        std::fs::write(dir.path().join(".credentials.json"), valid).unwrap();
        assert_eq!(
            account_session_status(&account),
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
