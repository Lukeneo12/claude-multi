---
# Spec: Inheritance status panel in Preferences

| Field | Value |
|-------|-------|
| **Date** | 2026-06-29 |
| **Author** | Lukeneo12 |
| **Status** | Draft |
| **Type** | Feature (multi-module: Rust backend + React frontend) |
| **Related PRD** | N/A |

---

## 1. Context / Problem

The inheritance feature (`inherit.rs`, branch `feat/inherit-user-config`, PR #3) links
user-level `~/.claude` resources — `agents`, `commands`, `skills`, `output-styles` — into
each account's isolated config dir. Today this is **entirely backend and implicit**:

- `ensure_account_inherits` (`commands.rs:36`) runs **at session launch/open**, not from the UI.
- The only user-facing surface is a **native Tauri dialog** (`prompt_inherit_decision`,
  `commands.rs:10`) that appears **only on conflict** (the account already has its own
  `skills/`, etc.) and asks Merge / Skip. The choice is persisted in
  `account.inherit_overrides` in the config JSON.
- The Preferences window (`App.tsx`, `api.ts`) has **zero** references to inheritance.
  There is no way to see what an account inherits, what is its own, or to review/change a
  past Merge/Skip decision without launching a session and re-triggering the dialog.

**Desired state:** a read-only-by-default panel in Preferences where, per account, the user
sees the inheritance status of each subdir and can flip the Merge/Skip decision on conflicted
subdirs without launching a session.

## 2. Goals / Non-goals

### Goals
- Add an **"Inheritance" card** to the Preferences window showing, for a selected account,
  one row per inheritable subdir (`agents`, `commands`, `skills`, `output-styles`) with a
  status badge: `inherited` / `skipped` / `conflict` / `none`.
- Expose a `Merge | Skip` toggle on **every row that has something to inherit** (not only
  `conflict` rows). The toggle writes `account.inherit_overrides` and re-applies inheritance.
- Make a persisted decision **sticky**: change `resolve_subdir` so `Some(Skip)` always skips
  and `Some(Merge)` always links, regardless of whether the account currently has own entries.
  This removes the "stale skip → re-prompt" branch so the UI toggle is coherent in any state.
- Reuse existing pure inheritance primitives (`plan_links` / `has_conflict`); the only logic
  change is the sticky-decision adjustment in `resolve_subdir`.
- Account selection via a dropdown inside the single Inheritance card (not per-account inline);
  selecting an account **auto-refreshes** its status (no explicit Refresh button).

### Non-goals
- **No per-entry listing** in v1 (no file-by-file `own` vs `linked` breakdown). Row-level
  status only.
- **No change to `plugins` handling** — it stays excluded by design (enablement lives in
  per-account `.claude.json`). The card shows a static "excluded by design" note, no controls.
- **No change to when inheritance is applied at launch** (`ensure_account_inherits` stays).
- **No removal of the native conflict dialog** — it remains the launch-time fallback for the
  one undecided case (`None` + conflict).
- No new config schema fields beyond what `inherit_overrides` already stores.

> **Note — behavior change:** making decisions sticky alters PR #3's current behavior. Today a
> `Skip` on a subdir whose own entries later disappear re-prompts; after this change it stays
> skipped until the user flips it in the panel. This is intentional and is the reason the panel
> exists (the user now manages decisions explicitly instead of via implicit re-prompts).

## 3. Acceptance Criteria

- [ ] AC1: `resolve_subdir` is changed so `Some(InheritDecision::Skip)` returns `SubdirPlan::Skip`
      and `Some(InheritDecision::Merge)` returns `SubdirPlan::Link(...)` **regardless of `has_conflict`**;
      the `Some(Skip)` + no-conflict → `NeedsPrompt` branch is removed. `None` keeps prompting only
      on conflict.
- [ ] AC2: The two existing stale-skip tests (`inherit.rs:275 test_should_reprompt_when_skip_decision_is_stale`,
      `inherit.rs:390 test_should_reprompt_when_skip_is_stale`) are rewritten to assert the new sticky
      behavior (skip persists without re-prompt), and `cargo test` passes.
- [ ] AC3: A new Tauri command `get_inherit_status(account_id: String) -> Vec<InheritSubdirStatus>`
      returns one entry per subdir in {`agents`, `commands`, `skills`, `output-styles`}, each with
      `{ subdir: String, status: InheritStatus, decision: Option<InheritDecision> }` where
      `InheritStatus ∈ { "inherited", "skipped", "conflict", "none" }`.
- [ ] AC4: Given `~/.claude/<subdir>` does not exist (or is empty), When `get_inherit_status` runs,
      Then that subdir's `status` is `"none"`.
- [ ] AC5: Status mapping, When source has entries:
      `decision == Skip` → `"skipped"`; `decision == Merge` OR (`None` AND no conflict) → `"inherited"`;
      `None` AND conflict → `"conflict"` (with `decision == None`).
- [ ] AC6: A new Tauri command `set_inherit_decision(account_id, subdir, decision)` persists the
      decision into `account.inherit_overrides`, re-runs `ensure_inherited`, and is allowed on **any**
      subdir (not only conflicted ones); calling `get_inherit_status` again reflects the new decision.
- [ ] AC7: The Preferences window renders an "Inheritance" card with an account `<select>`; choosing
      an account auto-fetches and shows one row per subdir with the correct status badge.
- [ ] AC8: Every row whose `status != "none"` shows a `Merge | Skip` toggle reflecting the current
      effective decision; clicking it calls `set_inherit_decision` and the row updates without a full
      page reload. `"none"` rows show no toggle.
- [ ] AC9: `get_inherit_status` never writes inside the default `~/.claude` (read/list only);
      verified by a unit test asserting no writes to the source dir.
- [ ] AC10: `cargo test`, `cargo clippy --all-targets -- -D warnings`, and `npm run build` (0 TS errors)
      all pass.

## 4. Approach

Four thin pieces:

**Backend — sticky decision (`inherit.rs`):**
- Change `resolve_subdir` (`inherit.rs:60`): `Some(Skip) -> SubdirPlan::Skip` and
  `Some(Merge) -> SubdirPlan::Link(...)` unconditionally; drop the `has_conflict` stale check in
  the `Some(Skip)` arm. The `None` arm is unchanged (prompt only on conflict).
- Rewrite the two stale-skip tests (`inherit.rs:275`, `inherit.rs:390`) to assert the new behavior.

**Backend — status query (`inherit.rs` + `commands.rs`):**
- Add a pure classifier in `inherit.rs`, e.g.
  `subdir_status(decision, source_entries, dest_entries) -> InheritStatus`, built on the same
  primitives (`has_conflict`, presence of `source_entries`). Mapping per AC4/AC5. Pure, unit-testable,
  no disk access (caller passes the already-listed entries).
- Add `#[tauri::command] get_inherit_status` in `commands.rs` that loads config, resolves the
  account's `config_dir` via `expand_tilde`, lists source/dest entries per subdir (read/list only),
  and returns the serializable `Vec<InheritSubdirStatus>`. No prompting, no writes.

**Backend — decision setter (`commands.rs`):**
- Extract the "persist a decision + re-apply" tail of `ensure_account_inherits`
  (`commands.rs:58-67`) into a reusable path, and expose `#[tauri::command] set_inherit_decision`
  that writes one subdir's decision into `inherit_overrides`, saves config, and calls
  `inherit::ensure_inherited`.

**Frontend (`api.ts` + `App.tsx` + `App.css`):**
- `api.ts`: add types (`InheritDecision`, `InheritStatus`, `InheritSubdirStatus`) and wrappers
  `getInheritStatus(accountId)` / `setInheritDecision(accountId, subdir, decision)`.
- `App.tsx`: new "Inheritance" `<section className="card">` with an account `<select>`, local
  state for the fetched statuses, a row per subdir with a status badge, and a `Merge | Skip`
  toggle on conflict rows. Reuse existing `card` / `row` / `select` / `btn` classes; add minimal
  badge styles in `App.css`.

### Key decisions
- **Sticky decisions over stale re-prompt**: a persisted `Skip`/`Merge` is always honored. Chosen so
  the UI toggle is coherent in any state; tradeoff is the PR #3 behavior change documented in Non-goals.
- **Toggle on every non-`none` row** (not just conflict): the user explicitly wants to force a decision
  even without a conflict (e.g. opt a whole subdir out of inheritance).
- **Single card + account dropdown** (not per-account inline expander): keeps the Accounts card
  uncluttered and matches the existing flat card layout. Tradeoff: one extra click to switch account.
- **Row-level status only in v1** (no entry listing): smaller backend payload and simpler UI;
  entry-level detail can be a v2 once the panel proves useful.
- **New read-only command instead of widening `get_config`**: keeps `Config` the single source of
  truth for persisted state; derived/computed inheritance status stays out of the saved JSON.
- **Reuse `ensure_inherited` for the setter** rather than a new write path: avoids duplicating the
  symlink/merge logic and keeps launch-time and UI-time behavior identical.

### Alternatives considered
- **Option A — show status inside the existing Accounts card rows:** rejected; crowds the row and
  the user picked the dedicated-card + selector layout.
- **Option B — compute status in the frontend by reading dirs via JS:** rejected; the frontend must
  not reach the filesystem, and the classification logic already lives (and is tested) in Rust.
- **Option C — fold status into `get_config`:** rejected; mixes persisted config with derived state
  and forces a recompute on every config load.

## 5. Risks / Rollback

### Risks
| Risk | Probability | Impact | Mitigation |
|------|-------------|--------|------------|
| Sticky-decision change has side effects elsewhere in launch flow | Med | Med | `resolve_subdir` is pure and centrally used by `ensure_inherited`; rewrite both stale tests (AC2) and run full `cargo test` |
| Status classification drifts from actual launch-time behavior | Med | Med | Build `subdir_status` on the **same** primitives `resolve_subdir`/`has_conflict` use; unit-test all four states |
| A write accidentally lands in `~/.claude` from the new query path | Low | High | AC9 test asserts source dir is untouched; command is read/list only |
| `set_inherit_decision` re-applies inheritance with stale config | Low | Med | Load → mutate → save → re-apply in one command, same order as `ensure_account_inherits` |
| Subdir name typo breaks the contract with `resolve_subdir` | Low | Med | Reuse the existing `INHERITED_SUBDIRS` constant in query, setter, and `ensure_inherited` |

### Rollback plan
Mostly additive. To revert: drop the two new commands from `tauri::generate_handler!` in `lib.rs`,
remove the Inheritance `<section>` from `App.tsx`, and remove the new wrappers/types from `api.ts`.
The one non-additive change is `resolve_subdir`; reverting restores the stale-prompt branch and its
two tests. No config schema migration is involved, so existing `inherit_overrides` data is unaffected.
`git revert` of the feature commit(s) fully restores prior state.

## 6. Open questions
_None — both prior questions resolved:_
- ✅ Toggle is available on **every non-`none` row**, not only conflicts (sticky decisions).
- ✅ Account dropdown **auto-refreshes** status on select; no explicit Refresh button.

---

*Spec generated with `/spec` skill. Update this file if the approach changes during implementation.*
