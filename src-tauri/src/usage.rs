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

use chrono::{DateTime, Duration, Utc};
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

/// Cost-proportional weights, relative to input. The ratios are the same across
/// Claude models (output 5×, cache-write 1.25×, cache-read 0.1× the input
/// price), so a weighted token sum is ~proportional to dollar/limit consumption
/// — unlike a raw sum, which is dominated by cheap, volatile cache reads and
/// would not track the subscription `% used`.
const W_OUTPUT: f64 = 5.0;
const W_CACHE_WRITE: f64 = 1.25;
const W_CACHE_READ: f64 = 0.1;

impl TokenTotals {
    /// Cost-proportional "usage units": the metric the tray shows and the user
    /// calibrates a ceiling against, so the percentage tracks real consumption.
    pub fn weighted_usage(&self) -> u64 {
        (self.input as f64
            + self.output as f64 * W_OUTPUT
            + self.cache_creation as f64 * W_CACHE_WRITE
            + self.cache_read as f64 * W_CACHE_READ)
            .round() as u64
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

/// Lower bound of the rolling 5-hour "session" window — a local proxy for the
/// subscription session block (whose real reset lives server-side).
pub fn session_window_start(now: DateTime<Utc>) -> DateTime<Utc> {
    now - Duration::hours(5)
}

/// Lower bound of the rolling 7-day "weekly" window — a local proxy for the
/// weekly subscription limit (the constraint that usually binds first).
pub fn week_window_start(now: DateTime<Utc>) -> DateTime<Utc> {
    now - Duration::days(7)
}

/// Compact human token count for a narrow tray label: `512`, `340k`, `1.2M`,
/// `5M` (a whole number of millions drops the trailing `.0`).
pub fn human_tokens(n: u64) -> String {
    if n < 1_000 {
        n.to_string()
    } else if n < 1_000_000 {
        format!("{:.0}k", n as f64 / 1_000.0)
    } else {
        let m = format!("{:.1}", n as f64 / 1_000_000.0);
        format!("{}M", m.strip_suffix(".0").unwrap_or(&m))
    }
}

/// Tray label for one window. With a calibrated ceiling, shows
/// `Session (5h): 1.2M / 5M · 24%`; without one, just `Session (5h): 1.2M tok`.
/// The percentage can exceed 100% (a ceiling is only an estimate).
pub fn format_window_line(name: &str, summary: &UsageSummary, limit: Option<u64>) -> String {
    let used = summary.tokens.weighted_usage();
    match limit {
        Some(cap) if cap > 0 => {
            let pct = (used as f64 / cap as f64 * 100.0).round() as u64;
            format!(
                "{name}: {} / {} · {pct}%",
                human_tokens(used),
                human_tokens(cap)
            )
        }
        _ => format!("{name}: {} tok", human_tokens(used)),
    }
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
        assert_eq!(s.messages, 2);
    }

    #[test]
    fn test_should_weight_tokens_by_cost_when_computing_usage() {
        let t = TokenTotals {
            input: 100,
            output: 10,
            cache_creation: 8,
            cache_read: 200,
        };
        // 100 + 10*5 + 8*1.25 + 200*0.1 = 100 + 50 + 10 + 20 = 180
        // (a raw sum would be 318, dominated by the 200 cache-read tokens).
        assert_eq!(t.weighted_usage(), 180);
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
        // Only the two in-window messages' input tokens (100 + 50) are summed.
        assert_eq!(s.tokens.input, 150);
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
        assert_eq!(s.tokens.input, 42);
    }

    #[test]
    fn test_should_return_zero_when_no_lines() {
        let s = aggregate_lines(std::iter::empty::<&str>(), epoch());
        assert_eq!(s, UsageSummary::default());
    }

    #[test]
    fn test_should_offset_window_starts_by_5h_and_7d() {
        let now = "2026-07-05T14:00:00.000Z".parse::<DateTime<Utc>>().unwrap();
        assert_eq!(
            session_window_start(now),
            "2026-07-05T09:00:00.000Z".parse::<DateTime<Utc>>().unwrap()
        );
        assert_eq!(
            week_window_start(now),
            "2026-06-28T14:00:00.000Z".parse::<DateTime<Utc>>().unwrap()
        );
    }

    #[test]
    fn test_should_format_human_tokens_when_across_magnitudes() {
        assert_eq!(human_tokens(0), "0");
        assert_eq!(human_tokens(512), "512");
        assert_eq!(human_tokens(1_000), "1k");
        assert_eq!(human_tokens(340_000), "340k");
        assert_eq!(human_tokens(1_200_000), "1.2M");
        assert_eq!(human_tokens(5_000_000), "5M");
    }

    fn summary_of(total: u64) -> UsageSummary {
        UsageSummary {
            tokens: TokenTotals {
                input: total,
                output: 0,
                cache_creation: 0,
                cache_read: 0,
            },
            messages: 1,
        }
    }

    #[test]
    fn test_should_show_percent_against_ceiling_when_limit_set() {
        let s = summary_of(1_200_000);
        assert_eq!(
            format_window_line("Session (5h)", &s, Some(5_000_000)),
            "Session (5h): 1.2M / 5M · 24%"
        );
        assert!(!format_window_line("Session (5h)", &s, Some(5_000_000)).contains('$'));
    }

    #[test]
    fn test_should_show_raw_tokens_when_no_limit() {
        let s = summary_of(1_200_000);
        assert_eq!(
            format_window_line("Week (7d)", &s, None),
            "Week (7d): 1.2M tok"
        );
        // A zero ceiling is treated as "unset" (no division by zero).
        assert_eq!(
            format_window_line("Week (7d)", &s, Some(0)),
            "Week (7d): 1.2M tok"
        );
    }

    #[test]
    fn test_should_allow_percent_over_100_when_over_ceiling() {
        let s = summary_of(6_000_000);
        assert_eq!(
            format_window_line("Session (5h)", &s, Some(5_000_000)),
            "Session (5h): 6M / 5M · 120%"
        );
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
        assert_eq!(s.tokens.input, 300); // 100 + 200 across both lines
        assert_eq!(s.tokens.output, 30); // 10 + 20
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
