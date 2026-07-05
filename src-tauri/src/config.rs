use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

/// How an account resolves a conflict between its own resources in a subdir and
/// the shared resources inherited from `~/.claude`. Persisted per subdir name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum InheritDecision {
    /// Link the shared entries in alongside the account's own (own entries win).
    Merge,
    /// Leave this subdir isolated; inherit nothing.
    Skip,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Account {
    pub id: String,
    pub label: String,
    pub config_dir: String,
    /// Persisted conflict resolutions, keyed by inherited subdir name
    /// (`agents`, `commands`, …). Absent key = undecided. `#[serde(default)]`
    /// keeps legacy configs (without the field) loading.
    #[serde(default)]
    pub inherit_overrides: HashMap<String, InheritDecision>,
}

impl Account {
    /// Returns the email this account is logged in as, by reading
    /// `<config_dir>/.claude.json` → `oauthAccount.emailAddress`. Returns `None`
    /// if the account is not logged in (file missing or field absent). Only ever
    /// reads inside the account's own config dir, never the default `~/.claude`.
    pub fn logged_in_email(&self) -> Option<String> {
        let path = crate::paths::expand_tilde(&self.config_dir).join(".claude.json");
        let contents = std::fs::read_to_string(path).ok()?;
        let value: serde_json::Value = serde_json::from_str(&contents).ok()?;
        value
            .get("oauthAccount")?
            .get("emailAddress")?
            .as_str()
            .map(str::to_string)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Project {
    pub id: String,
    pub label: String,
    pub path: String,
    /// The account this project is launched under (account id). Empty for
    /// legacy/unassigned projects, which then appear under no account.
    #[serde(default)]
    pub account: String,
}

/// Global, calibrated token ceilings for the tray usage lines. There is no
/// supported/local source for Anthropic's real subscription limits (they live
/// server-side and aren't persisted), so the user sets an approximate ceiling —
/// e.g. by cross-referencing `/usage`'s percentage once — and the tray renders
/// `used / ceiling · %`. `None` = no ceiling set (tray shows raw tokens). Global
/// rather than per-account because the plans are usually the same across
/// accounts; a per-account override can come later if needed.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct UsageLimits {
    /// Approximate token ceiling for the rolling 5-hour "session" window.
    #[serde(default)]
    pub session_tokens: Option<u64>,
    /// Approximate token ceiling for the rolling 7-day "weekly" window.
    #[serde(default)]
    pub weekly_tokens: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Config {
    pub terminal: String,
    pub accounts: Vec<Account>,
    pub projects: Vec<Project>,
    /// Token ceilings for the tray usage lines. `#[serde(default)]` keeps legacy
    /// configs (without the field) loading.
    #[serde(default)]
    pub usage_limits: UsageLimits,
}

impl Default for Config {
    fn default() -> Self {
        let default_terminal = if cfg!(target_os = "macos") {
            "terminal"
        } else if cfg!(target_os = "windows") {
            "wt"
        } else {
            "gnome-terminal"
        };
        Config {
            terminal: default_terminal.to_string(),
            accounts: vec![Account {
                id: "personal".into(),
                label: "Personal".into(),
                config_dir: "~/.claude-personal".into(),
                inherit_overrides: HashMap::new(),
            }],
            projects: vec![],
            usage_limits: UsageLimits::default(),
        }
    }
}

impl Config {
    pub fn load(path: &Path) -> Config {
        match std::fs::read_to_string(path) {
            Ok(s) => serde_json::from_str(&s).unwrap_or_else(|_| Config::default()),
            Err(_) => Config::default(),
        }
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        std::fs::write(path, json)
    }

    pub fn account(&self, id: &str) -> Option<&Account> {
        self.accounts.iter().find(|a| a.id == id)
    }

    pub fn project(&self, id: &str) -> Option<&Project> {
        self.projects.iter().find(|p| p.id == id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_seed_only_personal_account_when_default() {
        let c = Config::default();
        let ids: Vec<_> = c.accounts.iter().map(|a| a.id.as_str()).collect();
        assert_eq!(ids, vec!["personal"]);
        let personal = c.account("personal").unwrap();
        assert_eq!(personal.label, "Personal");
        assert_eq!(personal.config_dir, "~/.claude-personal");
    }

    #[test]
    fn test_should_roundtrip_when_saved_and_loaded() {
        let dir = std::env::temp_dir().join("cm_cfg_roundtrip");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.json");
        let original = Config::default();
        original.save(&path).unwrap();
        let loaded = Config::load(&path);
        assert_eq!(loaded.accounts.len(), original.accounts.len());
        assert_eq!(loaded.terminal, original.terminal);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_should_return_defaults_when_file_missing_or_invalid() {
        let loaded = Config::load(std::path::Path::new("/nonexistent/cm/config.json"));
        assert_eq!(loaded.accounts.len(), 1);
    }

    #[test]
    fn test_should_read_email_when_account_logged_in() {
        let dir = std::env::temp_dir().join("cm_email_loggedin");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join(".claude.json"),
            r#"{"oauthAccount":{"emailAddress":"someone@example.com"}}"#,
        )
        .unwrap();
        let account = Account {
            id: "x".into(),
            label: "X".into(),
            config_dir: dir.to_string_lossy().to_string(),
            inherit_overrides: HashMap::new(),
        };
        assert_eq!(
            account.logged_in_email().as_deref(),
            Some("someone@example.com")
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_should_return_none_when_account_not_logged_in() {
        let account = Account {
            id: "x".into(),
            label: "X".into(),
            config_dir: "/nonexistent/cm-account-dir".into(),
            inherit_overrides: HashMap::new(),
        };
        assert_eq!(account.logged_in_email(), None);
    }

    #[test]
    fn test_should_default_to_empty_inherit_overrides_when_account_created() {
        let c = Config::default();
        let personal = c.account("personal").unwrap();
        assert!(personal.inherit_overrides.is_empty());
    }

    #[test]
    fn test_should_roundtrip_inherit_overrides_when_saved_and_loaded() {
        let dir = std::env::temp_dir().join("cm_cfg_inherit_roundtrip");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.json");
        let mut original = Config::default();
        original.accounts[0]
            .inherit_overrides
            .insert("agents".to_string(), InheritDecision::Skip);
        original.save(&path).unwrap();
        let loaded = Config::load(&path);
        assert_eq!(
            loaded.accounts[0].inherit_overrides.get("agents"),
            Some(&InheritDecision::Skip)
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_should_serialize_decision_as_lowercase_when_saved() {
        assert_eq!(
            serde_json::to_string(&InheritDecision::Merge).unwrap(),
            "\"merge\""
        );
        assert_eq!(
            serde_json::to_string(&InheritDecision::Skip).unwrap(),
            "\"skip\""
        );
    }

    #[test]
    fn test_should_default_usage_limits_to_none_when_config_created() {
        let c = Config::default();
        assert_eq!(c.usage_limits, UsageLimits::default());
        assert_eq!(c.usage_limits.session_tokens, None);
        assert_eq!(c.usage_limits.weekly_tokens, None);
    }

    #[test]
    fn test_should_roundtrip_usage_limits_when_saved_and_loaded() {
        let dir = std::env::temp_dir().join("cm_cfg_usage_limits_roundtrip");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.json");
        let mut original = Config::default();
        original.usage_limits.session_tokens = Some(5_000_000);
        original.usage_limits.weekly_tokens = Some(40_000_000);
        original.save(&path).unwrap();
        let loaded = Config::load(&path);
        assert_eq!(loaded.usage_limits.session_tokens, Some(5_000_000));
        assert_eq!(loaded.usage_limits.weekly_tokens, Some(40_000_000));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_should_default_usage_limits_when_field_absent_in_json() {
        // Legacy config.json without the field must still load.
        let dir = std::env::temp_dir().join("cm_cfg_legacy_no_usage_limits");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.json");
        std::fs::write(
            &path,
            r#"{"terminal":"terminal","accounts":[],"projects":[]}"#,
        )
        .unwrap();
        let loaded = Config::load(&path);
        assert_eq!(loaded.usage_limits, UsageLimits::default());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_should_default_inherit_overrides_when_field_absent_in_json() {
        // Legacy config.json without the field must still load.
        let dir = std::env::temp_dir().join("cm_cfg_legacy_no_inherit");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.json");
        std::fs::write(
            &path,
            r#"{"terminal":"terminal","accounts":[{"id":"a","label":"A","config_dir":"~/.claude-a"}],"projects":[]}"#,
        )
        .unwrap();
        let loaded = Config::load(&path);
        assert_eq!(loaded.accounts[0].id, "a");
        assert!(loaded.accounts[0].inherit_overrides.is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }
}
