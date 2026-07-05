//! Per-account token usage aggregated from local Claude Code session logs.
//!
//! Each account keeps its transcripts under `<config_dir>/projects/**/*.jsonl`,
//! one JSON record per line. Assistant messages carry a `message.usage` object
//! with token counts and a top-level ISO-8601 `timestamp`. This module sums
//! those counts for a time window (the tray shows "today"), staying strictly
//! inside the account's own `config_dir` — the default `~/.claude` is never read
//! here (project invariant).
//!
//! Split mirrors the rest of the crate: a pure aggregation core (fully
//! unit-tested without touching the filesystem) plus a thin edge that walks the
//! account's project dir. A monetary/USD figure is intentionally out of scope
//! for now — a hardcoded price table would drift silently from Anthropic
//! pricing (see `docs/specs/2026-07-05/spec-show-account-usage-in-tray.md`).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::UNIX_EPOCH;

use chrono::{DateTime, Local, TimeZone, Utc};
use serde::Deserialize;

use crate::config::Account;
use crate::paths::expand_tilde;

/// Token counts summed across messages, split by kind so callers can format (or
/// later price) them. All four kinds count toward [`TokenTotals::total`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TokenTotals {
    pub input: u64,
    pub output: u64,
    pub cache_creation: u64,
    pub cache_read: u64,
}

impl TokenTotals {
    pub fn total(&self) -> u64 {
        self.input + self.output + self.cache_creation + self.cache_read
    }

    fn add(&mut self, other: &TokenTotals) {
        self.input += other.input;
        self.output += other.output;
        self.cache_creation += other.cache_creation;
        self.cache_read += other.cache_read;
    }
}

/// Aggregated usage for one account over one window.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct UsageSummary {
    pub tokens: TokenTotals,
    /// Assistant messages counted in the window (useful for "no activity" UX).
    pub messages: u64,
}

/// Minimal view of one transcript line. Everything else in the record is
/// ignored; a missing/renamed field just makes [`parse_line`] return `None`.
#[derive(Deserialize)]
struct Record {
    #[serde(default)]
    r#type: String,
    timestamp: Option<DateTime<Utc>>,
    message: Option<RecordMessage>,
}

#[derive(Deserialize)]
struct RecordMessage {
    usage: Option<RawUsage>,
}

#[derive(Deserialize)]
struct RawUsage {
    #[serde(default)]
    input_tokens: u64,
    #[serde(default)]
    output_tokens: u64,
    #[serde(default)]
    cache_creation_input_tokens: u64,
    #[serde(default)]
    cache_read_input_tokens: u64,
}

/// Parse one transcript line into `(timestamp, tokens)` if it is an assistant
/// message carrying usage. Returns `None` for any other record or malformed
/// JSON, so a half-written line from a live session never aborts aggregation.
fn parse_line(line: &str) -> Option<(DateTime<Utc>, TokenTotals)> {
    let rec: Record = serde_json::from_str(line).ok()?;
    if rec.r#type != "assistant" {
        return None;
    }
    let ts = rec.timestamp?;
    let usage = rec.message?.usage?;
    Some((
        ts,
        TokenTotals {
            input: usage.input_tokens,
            output: usage.output_tokens,
            cache_creation: usage.cache_creation_input_tokens,
            cache_read: usage.cache_read_input_tokens,
        },
    ))
}

/// Sum usage across `lines`, counting only messages whose timestamp is at or
/// after `since`. Malformed or non-assistant lines are skipped.
pub fn aggregate_lines<'a, I>(lines: I, since: DateTime<Utc>) -> UsageSummary
where
    I: IntoIterator<Item = &'a str>,
{
    let mut summary = UsageSummary::default();
    for line in lines {
        if let Some((ts, tokens)) = parse_line(line) {
            if ts >= since {
                summary.tokens.add(&tokens);
                summary.messages += 1;
            }
        }
    }
    summary
}

/// The instant local midnight (start of `now`'s day) occurs, as a UTC instant —
/// the lower bound of the "today" window. On a DST-ambiguous midnight takes the
/// earliest valid instant; on a nonexistent one degrades to `now`.
pub fn today_start(now: DateTime<Local>) -> DateTime<Utc> {
    let midnight = now
        .date_naive()
        .and_hms_opt(0, 0, 0)
        .expect("00:00:00 is always a valid time");
    Local
        .from_local_datetime(&midnight)
        .earliest()
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_else(|| now.with_timezone(&Utc))
}

/// Compact human token count for a narrow tray label: `512`, `340k`, `1.2M`.
pub fn human_tokens(n: u64) -> String {
    if n < 1_000 {
        n.to_string()
    } else if n < 1_000_000 {
        format!("{:.0}k", n as f64 / 1_000.0)
    } else {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    }
}

/// Tray label for the "today" window: tokens only, no cost. E.g. `Today: 1.2M tok`.
pub fn format_tray_label(summary: &UsageSummary) -> String {
    format!("Today: {} tok", human_tokens(summary.tokens.total()))
}

/// Aggregate an account's usage since `since`, reading only inside the account's
/// own `config_dir`. A missing dir yields a zeroed summary (never an error,
/// never a read of the default `~/.claude`).
pub fn account_usage(account: &Account, since: DateTime<Utc>) -> UsageSummary {
    let projects = expand_tilde(&account.config_dir).join("projects");
    let mut files = Vec::new();
    collect_jsonl(&projects, &mut files);

    let mut summary = UsageSummary::default();
    for path in files {
        if let Some(agg) = file_usage(&path, since) {
            summary.tokens.add(&agg.tokens);
            summary.messages += agg.messages;
        }
    }
    summary
}

/// Recursively collect `*.jsonl` files under `dir`. A missing or unreadable dir
/// contributes nothing (no panic, no error).
fn collect_jsonl(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_jsonl(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "jsonl") {
            out.push(path);
        }
    }
}

/// Per-file aggregate with an `(mtime, size)` memo cache. Files last modified
/// before `since` are skipped entirely (they can hold no message in the window),
/// which — combined with the cache — keeps repeated menu builds cheap.
fn file_usage(path: &Path, since: DateTime<Utc>) -> Option<UsageSummary> {
    let meta = std::fs::metadata(path).ok()?;
    let mtime = meta.modified().ok()?;

    // Prefilter: a file untouched since the window start holds nothing new.
    if DateTime::<Utc>::from(mtime) < since {
        return None;
    }

    let key = CacheKey {
        path: path.to_path_buf(),
        mtime_secs: mtime.duration_since(UNIX_EPOCH).ok()?.as_secs(),
        size: meta.len(),
        since_millis: since.timestamp_millis(),
    };
    if let Some(hit) = file_cache().lock().unwrap().get(&key).copied() {
        return Some(hit);
    }

    let contents = std::fs::read_to_string(path).ok()?;
    let summary = aggregate_lines(contents.lines(), since);
    file_cache().lock().unwrap().insert(key, summary);
    Some(summary)
}

/// Cache key: same file (path) + same bytes (mtime, size) + same window (since)
/// ⇒ same aggregate, so we can skip re-parsing across menu rebuilds.
#[derive(Clone, PartialEq, Eq, Hash)]
struct CacheKey {
    path: PathBuf,
    mtime_secs: u64,
    size: u64,
    since_millis: i64,
}

fn file_cache() -> &'static Mutex<HashMap<CacheKey, UsageSummary>> {
    static CACHE: OnceLock<Mutex<HashMap<CacheKey, UsageSummary>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap as StdHashMap;

    fn epoch() -> DateTime<Utc> {
        DateTime::<Utc>::from_timestamp(0, 0).unwrap()
    }

    fn assistant_line(ts: &str, input: u64, output: u64, cc: u64, cr: u64) -> String {
        format!(
            r#"{{"type":"assistant","timestamp":"{ts}","message":{{"model":"claude-opus-4-8","usage":{{"input_tokens":{input},"output_tokens":{output},"cache_creation_input_tokens":{cc},"cache_read_input_tokens":{cr}}}}}}}"#
        )
    }

    #[test]
    fn test_should_sum_all_token_kinds_when_aggregating() {
        let lines = [
            assistant_line("2026-07-05T10:00:00.000Z", 100, 10, 5, 20),
            assistant_line("2026-07-05T11:00:00.000Z", 200, 20, 5, 30),
        ];
        let s = aggregate_lines(lines.iter().map(String::as_str), epoch());
        assert_eq!(
            s.tokens,
            TokenTotals {
                input: 300,
                output: 30,
                cache_creation: 10,
                cache_read: 50,
            }
        );
        assert_eq!(s.tokens.total(), 390);
        assert_eq!(s.messages, 2);
    }

    #[test]
    fn test_should_exclude_messages_before_since_when_aggregating() {
        let since = "2026-07-05T00:00:00.000Z".parse::<DateTime<Utc>>().unwrap();
        let lines = [
            assistant_line("2026-07-04T23:59:59.000Z", 999, 999, 999, 999), // before
            assistant_line("2026-07-05T00:00:00.000Z", 100, 0, 0, 0),       // boundary in
            assistant_line("2026-07-05T09:00:00.000Z", 50, 0, 0, 0),        // in
        ];
        let s = aggregate_lines(lines.iter().map(String::as_str), since);
        assert_eq!(s.messages, 2);
        assert_eq!(s.tokens.total(), 150);
    }

    #[test]
    fn test_should_skip_malformed_and_non_assistant_lines() {
        let lines = [
            "{ this is not json".to_string(),
            "".to_string(),
            r#"{"type":"user","message":{"content":"hi"}}"#.to_string(),
            r#"{"type":"assistant","timestamp":"2026-07-05T10:00:00.000Z","message":{}}"#
                .to_string(), // no usage
            assistant_line("2026-07-05T10:00:00.000Z", 42, 0, 0, 0),
        ];
        let s = aggregate_lines(lines.iter().map(String::as_str), epoch());
        assert_eq!(s.messages, 1);
        assert_eq!(s.tokens.total(), 42);
    }

    #[test]
    fn test_should_return_zero_when_no_lines() {
        let s = aggregate_lines(std::iter::empty::<&str>(), epoch());
        assert_eq!(s, UsageSummary::default());
    }

    #[test]
    fn test_should_compute_local_day_start_as_utc() {
        let now = Local.with_ymd_and_hms(2026, 7, 5, 14, 30, 0).unwrap();
        let expected = Local
            .with_ymd_and_hms(2026, 7, 5, 0, 0, 0)
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(today_start(now), expected);
        // Invariant: the window start is never in the future and within a day.
        assert!(today_start(now) <= now.with_timezone(&Utc));
    }

    #[test]
    fn test_should_format_human_tokens_when_across_magnitudes() {
        assert_eq!(human_tokens(0), "0");
        assert_eq!(human_tokens(512), "512");
        assert_eq!(human_tokens(1_000), "1k");
        assert_eq!(human_tokens(340_000), "340k");
        assert_eq!(human_tokens(1_200_000), "1.2M");
    }

    #[test]
    fn test_should_render_today_tokens_only_when_formatting_label() {
        let s = UsageSummary {
            tokens: TokenTotals {
                input: 1_000_000,
                output: 200_000,
                cache_creation: 0,
                cache_read: 0,
            },
            messages: 3,
        };
        assert_eq!(format_tray_label(&s), "Today: 1.2M tok");
        // No monetary value in Phase 1.
        assert!(!format_tray_label(&s).contains('$'));
    }

    #[test]
    fn test_should_aggregate_account_usage_from_projects_dir() {
        let dir = std::env::temp_dir().join("cm_usage_account_ok");
        let proj = dir.join("projects").join("some-encoded-path");
        std::fs::create_dir_all(&proj).unwrap();
        let body = format!(
            "{}\n{}\n",
            assistant_line("2026-07-05T10:00:00.000Z", 100, 10, 0, 0),
            assistant_line("2026-07-05T11:00:00.000Z", 200, 20, 0, 0),
        );
        std::fs::write(proj.join("session.jsonl"), body).unwrap();

        let account = Account {
            id: "x".into(),
            label: "X".into(),
            config_dir: dir.to_string_lossy().to_string(),
            inherit_overrides: StdHashMap::new(),
        };
        let s = account_usage(&account, epoch());
        assert_eq!(s.messages, 2);
        assert_eq!(s.tokens.total(), 330);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_should_return_zero_when_config_dir_missing() {
        let account = Account {
            id: "x".into(),
            label: "X".into(),
            config_dir: "/nonexistent/cm-usage-missing".into(),
            inherit_overrides: StdHashMap::new(),
        };
        let s = account_usage(&account, epoch());
        assert_eq!(s, UsageSummary::default());
    }
}
