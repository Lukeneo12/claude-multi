# Spec: Show account usage in the tray

| Field | Value |
|-------|-------|
| **Date** | 2026-07-05 |
| **Author** | Lukeneo12 |
| **Status** | Draft |
| **Type** | Feature |
| **Related PRD** | N/A |

---

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

**Phase 1 (this spec, implemented now):**
- A pure `usage` module that, given an `Account`, aggregates token consumption from
  that account's `<config_dir>/projects/**/*.jsonl` for a given time window. The
  aggregation core is window-parameterized; the tray displays **today** only.
- The tray menu shows one **disabled** usage line per account, e.g.
  `Today: 1.2M tok`, positioned right under the existing status line — **tokens
  only, no cost** (see Non-goals).
- The usage line is shown **only for logged-in accounts**; a logged-out account
  (status `Unknown`) renders no `usage::` line at all.
- Reading is confined to each account's own `config_dir` — the default `~/.claude`
  is never written and never counted against an account (invariant preserved).
- Usage refreshes on menu build only (**on-open**); no periodic tick.
- Tray rebuild stays responsive: aggregation must not block the UI thread
  noticeably (see Approach → performance).
- Pure logic covered by unit tests (TDD), `cargo clippy -- -D warnings` clean.

**Phase 2 (sketch only, separate spec):**
- Add a subscription-limit source (percent of block used, reset time) as a second
  tray line, behind the same display contract.

### Non-goals
- **Any monetary / USD figure in Phase 1.** No price table, no `~$`. Cost is
  deferred until prices can be sourced reliably (Approach → *Deferred: cost
  estimation*), and if/when added would likely live in Preferences, not the tray.
- Real-time / streaming updates. Usage refreshes on menu build (**on-open**) only,
  not continuously and without a periodic tick.
- Windows beyond **today** in the tray label. The core may aggregate other windows
  (e.g. 7-day) for future/Preferences use, but the tray shows today only.
- Per-project breakdown in the tray (tray items are plain text). A richer
  breakdown in the Preferences window is out of scope for this spec.
- Phase 2 subscription-limits implementation (only sketched here).
- Historical charts, exports, or persistence of computed aggregates.

## 3. Acceptance Criteria

- [ ] **AC1:** Given an account whose `config_dir` contains
  `projects/<p>/<session>.jsonl` files with `assistant` messages carrying a
  `message.usage` object, when usage is aggregated, then the returned totals equal
  the sum of `input_tokens + output_tokens + cache_creation_input_tokens +
  cache_read_input_tokens` across all messages within the requested window.
- [ ] **AC2:** Given messages with timestamps, when aggregating for the "today"
  window, then only messages whose timestamp falls in the current local day are
  counted (messages from prior days are excluded), with correct boundaries around
  local midnight.
- [ ] **AC3:** Given an account whose `config_dir` does not exist or has no
  `projects/` dir, when usage is aggregated, then it returns a zeroed result and
  does **not** error, and does **not** read from the default `~/.claude`.
- [ ] **AC4:** Given an account that is **logged out** (`logged_in_email()` is
  `None`, i.e. status `Unknown`), when the tray menu is built, then **no**
  `usage::<account_id>` item is rendered for that account.
- [ ] **AC5:** For a **logged-in** account, the tray menu renders, under its status
  line, a disabled item whose id is `usage::<account_id>` and whose label reflects
  the current **today** aggregation in tokens only (e.g. `Today: 1.2M tok`, no
  monetary value). `parse_menu_id("usage::x")` round-trips to a usage action
  classified like other disabled/status ids.
- [ ] **AC6:** A malformed / partially-written `.jsonl` line is skipped without
  aborting aggregation of the rest of the file (robust to concurrent writes by a
  live session).
- [ ] **AC7:** `cargo test` passes and `cargo clippy --all-targets -- -D warnings`
  is clean; new pure logic follows `test_should_X_when_Y` naming.

## 4. Approach

### Overview

Add a single-responsibility module `src-tauri/src/usage.rs` that mirrors the
existing style split (pure aggregation core + thin edge I/O), then wire a display
line into `tray.rs`. `config`/`commands` stay the source of truth and glue.

```
usage.rs (new)
 ├─ pure:  UsageWindow, TokenTotals, UsageSummary
 ├─ pure:  aggregate_lines(lines, window, now) -> UsageSummary  # parse + window + sum
 ├─ pure:  format_tray_label(&UsageSummary) -> String           # "Today: 1.2M tok"
 └─ edge:  account_usage(&Account, window, now) -> UsageSummary  # walk projects/**/*.jsonl
```

### Data model (Phase 1)

- Read each line of every `<config_dir>/projects/**/*.jsonl`. Keep only records
  where `type == "assistant"` and `message.usage` is present.
- Extract `timestamp` (top-level ISO field already present in the logs — **TODO:
  confirm exact field name during implementation**, likely `timestamp`), `model`,
  and the four token counts.
- Bucket into `TokenTotals` for the requested `window` (tray uses `today`). Keep
  the aggregation window-parameterized so other windows can be added later without
  reshaping the core.

### Deferred: cost estimation (NOT in Phase 1)

Cost is out of scope for Phase 1 (see Non-goals) because a hardcoded price table
drifts silently from Anthropic's pricing and there is no stable public pricing API
to read at runtime. Documented here only as the intended future path:

- A `PriceTable` mapping `model` → per-million-token USD prices for the four token
  kinds (input, output, cache-write, cache-read), sourced from Anthropic's public
  pricing page, stored with a `PRICES_VERIFIED_ON` date constant.
- A staleness guard: if `PRICES_VERIFIED_ON` is older than a threshold, suppress or
  visibly mark the estimate rather than show a stale number.
- Unknown model → tokens counted, cost contribution `0`, flagged as partial.
- Likely surfaced in the Preferences window, not the always-visible tray line.

No Phase-1 code depends on this; `UsageSummary` carries token totals only.

### Tray wiring

- `build_menu` gains a `usage::<account_id>` **disabled** item inserted directly
  after the existing `status::<account>` item — **only for logged-in accounts**
  (skipped when `logged_in_email()` is `None`, matching AC4).
- `parse_menu_id("usage::<id>")` resolves to `MenuAction::Unknown`, exactly like
  the existing `status::<id>` item: disabled menu items emit no click event, so no
  dedicated variant or handler branch is needed (a covering unit test pins this).
- Label produced by `format_tray_label`, **today only, tokens only** (e.g.
  `Today: 1.2M tok`). Token count uses a compact human format (`1.2M`, `340k`).

### Performance

- Parsing every `.jsonl` on each menu build could be heavy for large histories.
  Mitigations, in order of preference:
  1. **mtime + size cache**: memoize per-file aggregates keyed by `(path, mtime,
     len)`; only re-parse changed files. Cache lives in a `Mutex`/`OnceCell` in the
     Tauri state, not persisted.
  2. **Window prefilter**: skip files whose mtime is older than the aggregation
     window (today) entirely for the windowed totals.
  3. If still slow, compute usage **off the UI thread** and update the tray via
     `refresh_tray` when ready, showing `Today: …` (loading) first.
- Phase 1 will implement at least (1)+(2); (3) only if measured latency warrants it.

### Phase 2 sketch (subscription limits — separate spec)

- New source `limits.rs`: read the account's OAuth token (macOS Keychain /
  `<config_dir>/.credentials.json`), call the usage endpoint `/usage` uses, map to
  `{ block_pct, weekly_pct, resets_at }`.
- Render as a second disabled line `usage::limit::<account_id>`.
- Explicitly out of scope here; flagged risks: non-public endpoint, per-account
  token isolation on macOS, token refresh/expiry handling.

### Key decisions

- **Decision 1 — Local logs as the Phase 1 source.** Robust, offline, and respects
  the "only read inside each `config_dir`" invariant. Trade-off: shows spend, not
  remaining limit — accepted, limits come in Phase 2.
- **Decision 2 — New `usage.rs` module, pure core + edge I/O.** Matches the repo's
  single-responsibility layout (`launcher`/`inherit`/`config`); keeps aggregation
  fully unit-testable without touching the filesystem.
- **Decision 3 — Disabled tray line, id `usage::<account>`.** Reuses the exact
  pattern of the existing `status::<account>` disabled item and its menu-id
  contract; minimal surface change in `tray.rs`.
- **Decision 4 — No cost in Phase 1 (tokens only).** A hardcoded price table drifts
  silently from Anthropic pricing and there is no stable public pricing API to read
  at runtime without breaking "fully local". A possibly-wrong `~$` is worse than
  none, so Phase 1 shows tokens only; cost is deferred with a dated-table + staleness
  guard when we tackle it (likely in Preferences).

### Alternatives considered

- **Option A — Shell out to `ccusage` / an external CLI.** Rejected: adds a runtime
  dependency and GUI-`PATH` problems (the app process lacks the user's shell PATH),
  and we already have the raw logs.
- **Option B — Subscription-limits API first (Phase 2 as Phase 1).** Rejected for
  the first ship: non-public endpoint + macOS per-account token isolation make it
  fragile; local logs deliver value immediately with far less risk.
- **Option C — Show usage only in Preferences window.** Rejected: the user wants
  the at-a-glance, cross-account view that only the tray gives.

## 5. Risks / Rollback

### Risks

| Risk | Probability | Impact | Mitigation |
|------|-------------|--------|------------|
| Log schema (`usage` fields, timestamp field, `type`) changes across Claude Code versions | Med | Med | Parse defensively, skip unknown lines (AC6); centralize field access in one place; unit tests on fixtures |
| Parsing large histories slows tray build | Med | Med | mtime+size cache, 7d mtime prefilter, optional off-thread compute (Approach → performance) |
| Cost estimate would diverge from real billing (if added) | — | — | Avoided in Phase 1: no cost shown. Deferred with dated table + staleness guard (Approach) |
| Reading another account's / default `~/.claude` logs by mistake | Low | High | Path always derived from `Account.config_dir` via `expand_tilde`; unit test asserts default dir is never touched (AC3) |
| Timezone / "today" boundary off-by-one | Med | Low | Use local-day boundaries explicitly; unit tests around midnight (AC2) |

### Rollback plan

- Feature is additive (a new module + two localized edits in `tray.rs`). To revert:
  `git revert` the feature commit(s), or remove the `usage::<account>` line from
  `build_menu` and drop the `usage` variant from `parse_menu_id` — the rest of the
  tray is unaffected. No persisted state or migrations to undo.

## 6. Open questions

_All product/UX questions resolved (2026-07-05). Remaining item is an implementation
detail verified during coding._

- [ ] Exact top-level **timestamp field name** in the `.jsonl` records (confirm
  `timestamp` vs other) — verify against real logs during implementation.

### Resolved

- **Timestamp field** → OK, confirm `timestamp` against real logs while coding.
- **Tray label content** → **today only, tokens only** (no cost). ✅
- **Cost / pricing table** → **deferred** out of Phase 1; when added, prices sourced
  from Anthropic's pricing page with a `PRICES_VERIFIED_ON` date + staleness guard. ✅
- **Logged-out accounts** → **hide** the usage line entirely (no `usage::` item). ✅
- **Refresh cadence** → **on-open only**, no periodic tick. ✅

---

*Spec generated with `/spec` skill. Update this file if the approach changes during implementation.*
