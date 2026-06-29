# Inherit user-level config into isolated accounts — Design / Spec

**Status:** Approved (design) — pending implementation plan
**Date:** 2026-06-29
**Branch:** `feat/inherit-user-config`

## Context / Problem

Sessions launched through the app run with `CLAUDE_CONFIG_DIR` pointed at a
per-account isolated directory (`~/.claude-<suffix>`) so each account's auth and
config stay separate. This is intentional and correct for credentials.

The side effect: Claude Code reads **user-level** `agents`, `commands`, `skills`,
and `output-styles` from `$CLAUDE_CONFIG_DIR/<sub>`. The isolated dirs only ever
contain auth/config, so sessions launched from the app see **none** of the
user-level agents/commands/skills the user maintains in the real `~/.claude`.
Project-level resources (`.claude/agents`, `.claude/commands` inside a repo) are
unaffected because they don't depend on the config dir.

Goal: make each account's session inherit the user-level resources from
`~/.claude` while keeping auth isolated, with no manual steps.

## Goals / Non-goals

### Goals
- Before each `launch_session` / `open_session` / `login_account`, ensure the
  account's `config_dir` sees the user-level resources from `~/.claude` for the
  subdirs: `agents`, `commands`, `skills`, `output-styles`.
- Use **file-level symlinks** (link each entry), so account-specific resources
  coexist with inherited ones and are never clobbered.
- Be **idempotent and self-healing**: because it runs on every launch, resources
  added to `~/.claude` later show up on the next launch automatically.
- When an account already has its own real entries in a subdir, **ask once**
  whether to also inherit the shared ones (merge) or keep that subdir isolated
  (skip), and **persist** the decision so launches stay silent afterward.
- Cross-OS: works on macOS/Linux natively; on Windows, degrade to a copy-based
  fallback when symlink creation fails (no privilege required).

### Non-goals
- A Preferences UI to view/edit inheritance decisions or conflicts (future).
- A manual "sync now" button (launch-time auto-sync is the only trigger).
- Making the **source** configurable — it is hardcoded to `~/.claude` (YAGNI).
- A global on/off toggle for inheritance (YAGNI; revisit if requested).
- Syncing anything beyond the four listed subdirs (e.g. `settings.json`,
  `CLAUDE.md`, `plugins`) — explicitly out of scope.

## Acceptance criteria

1. Launching a session for an account whose isolated dir has **no** `agents`
   subdir results in `<config_dir>/agents` existing with a symlink per entry of
   `~/.claude/agents`; same for `commands`, `skills`, `output-styles`.
2. Subdirs absent from `~/.claude` are silently skipped (nothing created).
3. Entries whose name **already exists** in the account subdir are left
   untouched (account entry wins); only non-colliding entries get a link.
4. Running a launch twice produces no changes the second time (idempotent) and
   no duplicate links or errors.
5. Adding a new entry to `~/.claude/agents` and launching again creates a link
   for it without touching anything else (self-healing).
6. When an account subdir contains **real (non-symlink) entries** and no decision
   is persisted, the user is prompted once (merge/skip); the choice is saved to
   config keyed by account+subdir; subsequent launches apply it without asking.
7. A persisted `skip` decision leaves that subdir untouched; a persisted `merge`
   decision applies the file-level merge silently.
8. `skills/` entries (directories) and `.md` entries (files) are both linked
   correctly (per-entry type detection), including on Windows.
9. On Windows, when symlink creation fails, the entry is **copied** instead, and
   the launch still proceeds. Copy is refreshed on each launch for entries not
   owned by the account.
10. Nothing is ever written **inside** `~/.claude`; only the account dirs are
    modified.
11. `cargo test` passes; `cargo clippy --all-targets -- -D warnings` is clean
    (cross-OS dead code gated with targeted `#[cfg_attr]`, never crate-wide).

## Approach

### Mechanism (uniform, single code path)
For each subdir `S` in `[agents, commands, skills, output-styles]` where
`~/.claude/S` exists:
1. Resolve the persisted decision for `(account, S)`. If `skip` → do nothing.
2. Ensure `<config_dir>/S` exists as a **real directory** (create if absent).
3. **Conflict detection:** if `<config_dir>/S` contains any **real**
   (non-symlink) entry and no decision is persisted → this subdir needs a prompt.
4. **Link plan:** for each entry in `~/.claude/S` whose name does **not** exist in
   `<config_dir>/S`, create a symlink (`<config_dir>/S/<name>` → `~/.claude/S/<name>`).
   - Per-entry type detection: link as directory (skills) or file (`.md`).
   - On Windows, on symlink failure, **copy** the entry instead; refresh copies
     of entries not owned by the account on each launch.

The "no conflict / empty-or-absent subdir" case (the user's current situation)
needs no decision: links are created automatically, nothing is persisted (the
operation is idempotent, so re-deriving each launch is fine).

### Module layout (single-responsibility, pure core + I/O at edges)
New module **`inherit.rs`**, mirroring the `launcher.rs` pattern (pure builders +
edge I/O):

- **Pure (unit-tested):**
  - `plan_links(source_entries, dest_entries) -> Vec<LinkAction>` — which links to
    create, skipping name collisions.
  - `has_conflict(dest_entries) -> bool` — true if any dest entry is a real
    non-symlink entry.
- **I/O at edges (tempfile-based tests):**
  - `create_link(src, dst)` — symlink helper, `#[cfg]` per OS, file vs dir per
    entry; Windows copy fallback on failure.
  - `ensure_inherited(source, account, &decisions) -> Outcome` — reads dir
    entries, applies plan; returns `Outcome` that may include
    `NeedsPrompt(Vec<subdir>)` for unresolved conflicts.

- **`config.rs`:** `Account` gains `#[serde(default)] inherit_overrides:
  HashMap<String, InheritDecision>` (`subdir -> Merge | Skip`). Only conflict
  resolutions are stored.
- **`commands.rs` (glue):** before `build_script`/`spawn`, call `ensure_inherited`.
  If `NeedsPrompt`, show a blocking native dialog (existing `dialog` plugin) per
  subdir, record the decision, `config.save`, then re-apply.
- **Source:** hardcoded `~/.claude` via `expand_tilde`.

### Data flow
`launch_session` / `open_session` / `login_account`
→ `inherit::ensure_inherited(~/.claude, account, &account.inherit_overrides)`
→ if `NeedsPrompt`: dialog → persist decision → save config → re-apply
→ proceed to `build_script` → `write_script` → `adapters::spawn`.

### Invariant update
The CLAUDE.md invariant *"Never read/write the default `~/.claude`"* is reworded
to permit a **sanctioned read-only dependency**: the app may **list** and
**symlink-target** `~/.claude/<sub>`, but must **never write inside** `~/.claude`.
All created links/copies live under the account dirs. This wording change is part
of the implementation.

## Risks / Rollback

- **Windows without symlink privilege:** handled by the copy fallback (no
  privilege, no extra crates). Copy doesn't reflect in-place edits of an
  already-copied source entry until the next launch, and "copied-by-us" vs
  "account-owned" is fuzzier than with self-identifying symlinks; mitigated by
  refreshing non-account-owned copies each launch. Primary dev target is macOS.
- **Stale `skip` decision:** if the user later removes their account-specific
  entries, a persisted `skip` keeps skipping until changed. Editing decisions is
  a future Preferences feature. Documented limitation.
- **Reading `~/.claude`:** deliberate, sanctioned, read-only (see invariant
  update). Never writes there.
- **Rollback:** the feature is isolated in `inherit.rs` plus a single call site in
  `commands.rs`; reverting = removing the call. Created links/copies live only in
  account dirs and can be deleted manually without affecting `~/.claude`.
- **Testing:** pure core via unit tests (`test_should_X_when_Y`); I/O via
  `tempfile`; `clippy -D warnings` clean.
