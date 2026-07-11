# Spec: Seed `settings.json` into fresh account config dirs

| Field | Value |
|-------|-------|
| **Date** | 2026-07-11 |
| **Author** | Lukeneo12 |
| **Status** | Draft |
| **Type** | Feature |
| **Related PRD** | N/A (supersedes one non-goal of `docs/superpowers/specs/2026-06-29-inherit-user-config-design.md`) |

---

## 1. Context / Problem

Each claude-multi account runs Claude Code with an isolated `CLAUDE_CONFIG_DIR`
(`~/.claude-<suffix>`). The inheritance mechanism (`inherit.rs`) links the four
user-level resource subdirs (`agents`, `commands`, `skills`, `output-styles`)
from `~/.claude` into each account dir, but `settings.json` was explicitly out
of scope in the original design.

`settings.json` is where user-level Claude Code settings live — notably
`statusLine`, theme, and model preferences. Because it is never carried over,
every fresh account starts with no statusline (and none of the user's other
settings), and the user must reconfigure each account by hand. This was hit in
practice: the statusline silently didn't appear in sessions launched through
claude-multi until `settings.json` was fixed manually in the account dir.

Desired state: a fresh account dir picks up the user's `~/.claude/settings.json`
automatically, once, and then owns its copy.

## 2. Goals / Non-goals

### Goals
- On session/login launch for an account, if `<config_dir>/settings.json` does
  not exist and `~/.claude/settings.json` does, copy it into the account dir
  (a real file copy — never a symlink).
- One-shot semantics: an existing `<config_dir>/settings.json` is never
  overwritten, merged, or re-synced. Accounts diverge freely after seeding.
- Pure decision logic covered by unit tests, following the existing
  `inherit.rs` split (pure planning + edge I/O).

### Non-goals
- Continuous sync or merge of `settings.json` changes from `~/.claude` after
  the initial seed.
- Filtering or transforming keys during the copy (e.g. stripping `hooks`,
  `permissions`, `env`). The file is copied wholesale.
- Seeding any other root-level file (`CLAUDE.md`, `.claude.json`, keybindings)
  — still out of scope.
- An Inheritance-panel row or conflict prompt for `settings.json` (there is no
  conflict: existing file always wins silently).

## 3. Acceptance Criteria

- [ ] AC1: Given `~/.claude/settings.json` exists and `<config_dir>/settings.json`
      does not, when a session, login, or re-login is launched for that account,
      then `<config_dir>/settings.json` exists afterwards as a regular file
      (not a symlink) with byte-identical content to the source.
- [ ] AC2: Given `<config_dir>/settings.json` already exists (any content),
      when a session is launched, then the file's content and mtime-relevant
      state are unchanged (no overwrite, no merge, no prompt).
- [ ] AC3: Given `~/.claude/settings.json` does not exist, when a session is
      launched, then no `settings.json` is created in the account dir and the
      launch succeeds (silent no-op).
- [ ] AC4: Nothing is ever written under `~/.claude` (read-only source).
- [ ] AC5: An I/O failure while copying does not abort the launch; the session
      still starts (settings seeding is best-effort, consistent with
      inheritance being a convenience layer). <!-- TODO: confirm — alternative
      is failing the launch loudly like ensure_inherited does today -->
- [ ] AC6: `cargo test` covers the pure seed-decision logic
      (`test_should_X_when_Y` naming) and `cargo clippy --all-targets -- -D warnings`
      stays clean.

## 4. Approach

Extend `inherit.rs` with a small, self-contained seeding step and call it from
the existing per-launch hook in `commands.rs`.

1. **`inherit.rs`** — add:
   - `pub const SEEDED_FILES: &[&str] = &["settings.json"];` (a list, so a
     future file costs one entry).
   - Pure planner `plan_file_seeds(source_files: &[String], dest_files: &[String]) -> Vec<String>`
     returning the file names to copy: present in source, absent in dest.
     Unit-tested exhaustively (both present, only source, only dest, neither).
   - Edge I/O `ensure_seeded(source: &Path, config_dir: &Path) -> io::Result<()>`
     that stats the real filesystem, calls the planner, and performs each copy
     via `std::fs::copy` after ensuring `config_dir` exists. Copy is
     dest-absent-checked immediately before copying to keep the window small.
2. **`commands.rs`** — in `ensure_account_inherits` (the single choke point
   already invoked by `launch_session`, `open_session`, `login_account`,
   `relogin_account`), call `inherit::ensure_seeded(&source, &config_dir)`
   right before `ensure_inherited`. No new Tauri command, no config schema
   change, no tray change.
3. **Docs** — update `CLAUDE.md` module table (`inherit.rs` row), add a line to
   `CHANGELOG.md`, add a manual step to `docs/SMOKE-CHECKLIST.md`, and amend the
   2026-06-29 design spec's non-goals with a pointer to this spec.

### Key decisions
- **Copy, not symlink:** Claude Code writes to `settings.json`; a symlink would
  make every account share (and race on) the user's real file, violating the
  "never write inside `~/.claude`" invariant the moment any session saves a
  setting. A copy keeps accounts fully isolated after the seed.
- **Seed at launch, not at account creation:** launch time is the existing
  trigger for inheritance (`ensure_account_inherits`), covers accounts created
  before this feature ships, and requires no changes to the Preferences flow.
- **Wholesale copy, no key filtering:** simplest correct behavior; the user can
  edit the per-account copy afterwards. Filtering would require opinions about
  which keys are "account-local" that we don't have evidence for yet.

### Alternatives considered
- **Option A: add `settings.json` to the per-entry symlink inheritance** —
  rejected: symlinked settings are shared-mutable state across accounts and
  would let a session write through to `~/.claude` (invariant violation).
- **Option B: seed at account creation in the Preferences UI (`save_config`)** —
  rejected: misses pre-existing accounts and duplicates the trigger; launch
  time already funnels through one function.
- **Option C: templated/filtered copy (strip `hooks`, `permissions`)** —
  rejected for now (YAGNI): adds JSON-parsing failure modes for an unproven
  need; revisit if seeded hooks cause real problems.

## 5. Risks / Rollback

### Risks
| Risk | Probability | Impact | Mitigation |
|------|-------------|--------|------------|
| Seeded `hooks`/`permissions`/`env` reference user-specific paths and misbehave in an account context | Med | Low | One-shot copy: user edits or deletes the per-account file once; document in SMOKE-CHECKLIST/CHANGELOG |
| User expects later `~/.claude/settings.json` edits to propagate (they won't) | Med | Low | State one-shot semantics in CHANGELOG; future "re-seed" affordance only if requested |
| Copy failure (perms, disk) breaks session launch | Low | Med | Best-effort per AC5 (or explicit error — open question) |

### Rollback plan
Revert the commit(s); the feature is additive with no config-schema or
menu-id changes. Already-seeded accounts keep their `settings.json` — harmless,
and removable by deleting `~/.claude-<suffix>/settings.json` manually.

## 6. Open questions
- [ ] AC5: should a copy failure be best-effort (log-and-continue) or fail the
      launch like `ensure_inherited` errors do today? Proposal: best-effort.

---

*Spec generated with `/spec` skill. Update this file if the approach changes during implementation.*
