# Spec: Show account usage in the tray

| Field | Value |
|-------|-------|
| **Date** | 2026-07-05 |
| **Author** | Lukeneo12 |
| **Status** | Draft |
| **Type** | Feature |
| **Related PRD** | N/A |

---

> **Revision (2026-07-05, during implementation):** the first cut shipped a
> `Today: <N> tok` line. In review it proved low-value — the user needs *how much
> of the session limit is left*, not raw spend. We investigated the
> subscription-limit source and confirmed it is **not obtainable locally or via
> any supported interface** (see §4 → *Investigation*). Product chose a **local
> rolling-window proxy**: two lines, `Session (5h)` and `Week (7d)`, each a
> cost-weighted token sum over the window vs. a user-calibrated **per-account ceiling**,
> rendered `used / ceiling · %`. This section's original two-source framing is kept
> for history; the Goals/AC/Approach below reflect the shipped design.

## 1. Context / Problem

`claude-multi` launches interactive Claude Code sessions under several isolated
accounts, each pinned to its own `CLAUDE_CONFIG_DIR` (`~/.claude-<suffix>`). The
tray menu already surfaces a per-account **status line** (the logged-in email, or
`Unknown` when logged out) via `build_menu` → `status::<account>`.

Today there is **no visibility into how much each account has consumed**. To know
whether an account is close to its limits, the user has to open a session and run
`/usage`. That is friction, and it doesn't give an at-a-glance, cross-account view
— the whole point of a multi-account tray app.

**Desired state:** each account in the tray shows a compact **usage line**
alongside its status line, so the user can eyeball consumption per profile without
launching anything.

Two data sources are possible, with very different robustness profiles:

- **Local consumption** — parse the session logs
  (`<config_dir>/projects/**/*.jsonl`), which already contain per-message `usage`
  (`input_tokens`, `output_tokens`, `cache_creation_input_tokens`,
  `cache_read_input_tokens`, `model`). Fully local, no network, aligns with the
  repo invariant of only reading inside each account's `config_dir`. Answers
  *"how much did I spend"*, not *"how much is left"*.
- **Subscription limits** — the numbers `/usage` shows (percentage of the 5-hour
  block, weekly limit, reset time). Server-side; requires an **authenticated,
  non-public API call** per account, using the OAuth token (Keychain on macOS,
  `<config_dir>/.credentials.json` elsewhere). Nicer to look at, but fragile and
  operationally harder (the macOS Keychain entry is global, not per-`config_dir`).

**Decision (agreed with product):** ship in **two phases** — Phase 1 delivers the
local-consumption line in the tray; Phase 2 adds the subscription-limit line as a
second, independent data source. This spec fully specifies **Phase 1** and sketches
**Phase 2** as a follow-up; Phase 2 gets its own spec before implementation.

**Cost is intentionally excluded from Phase 1.** A hardcoded price table would go
stale silently whenever Anthropic changes pricing, and there is no stable public
pricing API to read at runtime without breaking the "fully local" goal. Showing a
possibly-wrong `~$` is worse than showing none. Phase 1 therefore displays **tokens
only**; a monetary estimate is deferred until we can source prices reliably (see
Approach → *Deferred: cost estimation*).

## 2. Goals / Non-goals

### Goals

- A pure `usage` module that, given an `Account` and a window lower-bound, sums
  token consumption from that account's `<config_dir>/projects/**/*.jsonl`.
- The tray shows, **per logged-in account**, two **disabled** informational lines
  under the status line: `Session (5h): …` (rolling 5-hour window) and
  `Week (7d): …` (rolling 7-day window), proxying the subscription's session and
  weekly limits.
- The displayed number is a **cost-weighted** token sum (output 5×, cache-write
  1.25×, cache-read 0.1× relative to input), so it tracks real consumption rather
  than being dominated by cheap, volatile cache reads.
- A **per-account token ceiling** per window (`Account.usage_limits.session_tokens`,
  `…weekly_tokens`, both `Option<u64>`), editable in Preferences. Per-account
  because plans differ across accounts (a work account may allow far more than a
  personal one). With a ceiling set the line reads `used / ceiling · %` (percent may
  exceed 100%); without one it reads `used tok`.
- A logged-out account (status `Unknown`) renders **no** usage lines.
- Reading is confined to each account's own `config_dir` — the default `~/.claude`
  is never written and never counted (invariant preserved).
- Usage refreshes on menu build only (**on-open**); no periodic tick.
- Tray rebuild stays responsive (see Approach → performance).
- Pure logic covered by unit tests (TDD), `cargo clippy -- -D warnings` clean.

### Non-goals
- **Anthropic's real subscription limits / reset times.** Confirmed not available
  locally or via any supported interface (§4 → *Investigation*). We ship a local
  *proxy*, not the authoritative `% used`. The ceiling is a user-calibrated
  estimate, not a fetched limit.
- **Any monetary / USD figure.** The cost weighting shapes the token number but no
  `$` is shown; a real cost estimate would need a maintained price table (deferred).
- **Auto-scaling the ceiling from the account's plan tier.** The tier *is* readable
  locally (`oauthAccount.organizationRateLimitTier`, e.g. `default_claude_max_5x`),
  but it isn't reliably numeric across accounts (seen: `default_raven`), so we don't
  derive ceilings from it — the user calibrates per account. (Possible future hint.)
- Real-time / streaming updates (on-open refresh only, no periodic tick).
- Per-project breakdown in the tray; historical charts, exports, or persistence of
  computed aggregates.
- Calling any Anthropic endpoint or reading OAuth credentials.

## 3. Acceptance Criteria

- [ ] **AC1:** Given an account whose `config_dir` contains
  `projects/<p>/<session>.jsonl` files with `assistant` messages carrying a
  `message.usage` object, when usage is aggregated for a window, then the raw
  totals equal the sum of `input_tokens + output_tokens +
  cache_creation_input_tokens + cache_read_input_tokens` across all messages whose
  `timestamp` is ≥ the window lower-bound.
- [ ] **AC2:** The displayed usage equals the **cost-weighted** sum
  `input + 5·output + 1.25·cache_creation + 0.1·cache_read` (rounded), so cache
  reads do not dominate.
- [ ] **AC3:** `session_window_start(now) == now − 5h` and
  `week_window_start(now) == now − 7d`; messages before the bound are excluded.
- [ ] **AC4:** Given an account whose `config_dir` does not exist or has no
  `projects/` dir, when usage is aggregated, then it returns a zeroed result and
  does **not** error, and does **not** read from the default `~/.claude`.
- [ ] **AC5:** Given a logged-out account (`logged_in_email()` is `None`), the tray
  renders **no** `usage::…::<account_id>` items for it.
- [ ] **AC6:** For a logged-in account, the tray renders two disabled items,
  `usage::session::<id>` and `usage::week::<id>`, under the status line. With a
  ceiling the label is `<name>: <used> / <ceiling> · <pct>%` (pct may exceed 100);
  with `None` or `0` ceiling it is `<name>: <used> tok`. Both ids round-trip
  through `parse_menu_id` to `MenuAction::Unknown` (disabled items emit no event).
- [ ] **AC7:** `Account.usage_limits` (`session_tokens`, `weekly_tokens`:
  `Option<u64>`) round-trips through save/load, defaults to `None` per account, and
  legacy accounts without the field still load; the Preferences UI edits both per
  account and empty ⇒ `null`.
- [ ] **AC8:** A malformed / partially-written `.jsonl` line is skipped without
  aborting aggregation of the rest of the file (robust to concurrent live writes).
- [ ] **AC9:** `cargo test` passes and `cargo clippy --all-targets -- -D warnings`
  is clean; `npm run build` has 0 TS errors; new pure logic follows
  `test_should_X_when_Y` naming.

## 4. Approach

### Overview

Add a single-responsibility module `src-tauri/src/usage.rs` that mirrors the
existing style split (pure aggregation core + thin edge I/O), then wire a display
line into `tray.rs`. `config`/`commands` stay the source of truth and glue.

```
usage.rs (new)
 ├─ pure:  TokenTotals { weighted_usage() }, UsageSummary
 ├─ pure:  aggregate_lines(lines, since) -> UsageSummary          # parse + filter ≥since + sum
 ├─ pure:  session_window_start(now) / week_window_start(now)     # now−5h / now−7d
 ├─ pure:  human_tokens(n) / format_window_line(name, &summary, Option<u64>)
 └─ edge:  account_usage(&Account, since) -> UsageSummary         # walk projects/**/*.jsonl
config.rs:  Account.usage_limits: UsageLimits { session_tokens, weekly_tokens: Option<u64> }
```

### Data model

- Read each line of every `<config_dir>/projects/**/*.jsonl`. Keep only records
  where `type == "assistant"` and `message.usage` is present.
- Extract the top-level `timestamp` (confirmed against real logs: RFC-3339 UTC,
  e.g. `2026-06-30T18:23:25.694Z`) and the four token counts.
- Sum into `TokenTotals` for messages with `timestamp ≥ since`; `weighted_usage()`
  applies the cost weights and is what the tray shows.

### Investigation: why the real limit isn't used

Verified (incl. a `claude-code-guide` consult) that Anthropic's session/weekly
`% used` + reset (what `/usage` shows):
- has **no** headless CLI, JSON mode, hook, statusline, or SDK surface;
- is **not** persisted to any local file (`.claude.json` caches don't hold it);
- lives server-side, reachable only via an **undocumented** endpoint with each
  account's OAuth token (Keychain on macOS — a single global entry, so not cleanly
  per-account; reading it is sensitive and gated).

Confirmed by searching all of the user's transcripts and configs (only strings
from this very conversation matched). One useful thing *is* local: each account's
plan tier at `oauthAccount.organizationRateLimitTier` (e.g. `default_claude_max_5x`)
— but it isn't reliably numeric (also seen: `default_raven`), so it can't drive an
auto-scaled ceiling; it's at most a future calibration hint.

That endpoint path is fragile, credential-handling, and ToS-gray, so product chose
the local proxy. The rolling windows + cost weighting approximate the shape of the
limits; the user calibrates a **per-account** ceiling against `/usage` once.

### Tray wiring

- For each **logged-in** account, `build_menu` inserts two **disabled** items after
  `status::<id>`: `usage::session::<id>` and `usage::week::<id>` (skipped entirely
  when `logged_in_email()` is `None`).
- Windows computed once per build from `chrono::Utc::now()`.
- `parse_menu_id("usage::session::<id>")` / `("usage::week::<id>")` resolve to
  `MenuAction::Unknown` (disabled items emit no event; no handler branch needed).
- Labels via `format_window_line`, using `cfg.usage_limits.{session,weekly}_tokens`
  as the ceiling. Compact human format (`1.2M`, `340k`, `5M`).

### Performance

- Parsing every `.jsonl` on each menu build could be heavy for large histories.
  Mitigations, in order of preference:
  1. **mtime + size cache**: memoize per-file aggregates keyed by `(path, mtime,
     len)`; only re-parse changed files. Cache lives in a `Mutex`/`OnceCell` in the
     Tauri state, not persisted.
  2. **Window prefilter**: skip files whose mtime is older than the window
     lower-bound entirely (they can hold no message in the window).
  3. If still slow, compute usage **off the UI thread** and update the tray via
     `refresh_tray` when ready.
- Shipped: (1)+(2). Measured fine on real data (a 7-day window over a ~1000-message
  account parses in well under menu-build latency); (3) only if it regresses.

### Key decisions

- **Decision 1 — Local logs, no server call.** Robust, offline, respects the "only
  read inside each `config_dir`" invariant, handles no credentials. Trade-off: it's
  a proxy for the limit, not the authoritative number — accepted (see Investigation).
- **Decision 2 — Two rolling windows (5h / 7d).** Mirror the subscription's session
  and weekly limits; the weekly is usually the binding one. Both are free to compute
  from the same logs.
- **Decision 3 — Cost-weighted token metric.** Raw sums are dominated by cache reads
  (cheap, volatile) and wouldn't track `% used`. Weights `output 5× / cache-write
  1.25× / cache-read 0.1×` mirror the (model-invariant) pricing ratios, so the
  number is ~proportional to cost/limit consumption.
- **Decision 4 — Per-account, user-calibrated ceiling.** Anthropic's real limit
  isn't locally available; the user sets a ceiling per window **per account** (plans
  differ — a work account may allow far more than a personal one) and calibrates
  against `/usage`. `None` ⇒ show raw usage. (The plan tier is locally readable but
  not reliably numeric, so it isn't used to auto-scale — see Investigation.)
- **Decision 5 — New `usage.rs`, pure core + edge I/O; disabled tray items.** Matches
  the repo layout (`launcher`/`inherit`/`config`) and the `status::<id>` menu-id
  pattern; minimal surface change.

### Alternatives considered

- **Undocumented usage endpoint + per-account OAuth token.** The only route to the
  *real* `% used`. Rejected: undocumented (breaks without notice), credential-handling
  (macOS Keychain is a single global entry — not cleanly per-account), ToS-gray.
- **Raw (unweighted) token sum.** Rejected: dominated by cache reads, so it tracks
  caching behaviour rather than consumption; a calibrated ceiling would drift as the
  token-kind mix varies.
- **Shell out to `ccusage` / an external CLI.** Rejected: runtime dependency + the
  app process lacks the user's shell `PATH`; we already have the raw logs.
- **Show usage only in Preferences.** Rejected: the value is the at-a-glance,
  cross-account tray view.

## 5. Risks / Rollback

### Risks

| Risk | Probability | Impact | Mitigation |
|------|-------------|--------|------------|
| Log schema (`usage` fields, timestamp, `type`) changes across Claude Code versions | Med | Med | Parse defensively, skip unknown lines (AC8); field access centralized in `usage.rs`; fixture tests |
| Proxy diverges from the real `% used` (weighting/limit are estimates) | High | Low | Documented as a proxy (Non-goals); cost weighting keeps it directional; ceiling is recalibratable |
| Parsing large histories slows tray build | Med | Med | mtime+size memo cache + window mtime prefilter (Approach → performance); verified fine on real data |
| Reading another account's / default `~/.claude` logs by mistake | Low | High | Path always derived from `Account.config_dir` via `expand_tilde`; test asserts default dir untouched (AC4) |

### Rollback plan

- Feature is additive (a new `usage.rs`, a `usage_limits` config field, two `tray.rs`
  edits, one Preferences card). To revert: `git revert` the feature commit(s). The
  `usage_limits` field is `#[serde(default)]`, so old/new configs interoperate; no
  migrations to undo.

## 6. Open questions

_All product/UX questions resolved during implementation (2026-07-05)._

- **Timestamp field** → confirmed `timestamp` (RFC-3339 UTC) against real logs. ✅
- **What to show** → session-limit "remaining", not raw spend. Real limit isn't
  locally available → local rolling-window proxy. ✅
- **Windows** → both **Session (5h)** and **Week (7d)**. ✅
- **Ceiling** → **per-account** (one per window each), user-calibrated; `None` ⇒ raw. ✅
- **Metric** → **cost-weighted** tokens (not raw), so it tracks consumption. ✅
- **Logged-out accounts** → **hide** the usage lines. ✅
- **Refresh cadence** → **on-open only**, no periodic tick. ✅

### Possible follow-ups (not in this spec)
- Show each account's plan tier (`organizationRateLimitTier`) next to its ceiling
  inputs as a calibration hint.
- Auto-suggest a ceiling from a one-time `/usage` percentage the user pastes in.

---

*Spec generated with `/spec` skill. Update this file if the approach changes during implementation.*
