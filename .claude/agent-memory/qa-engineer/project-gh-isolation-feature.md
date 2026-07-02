---
name: project-gh-isolation-feature
description: QA review status of the per-account GH_CONFIG_DIR isolation feature (spec 2026-07-02) — AC coverage, edge cases, known low-risk quirks
metadata:
  type: project
---

Feature: `src-tauri/src/launcher.rs` exports `GH_CONFIG_DIR='<config_dir>/gh'`
(POSIX) / `$env:GH_CONFIG_DIR = '<config_dir>\gh'` (PowerShell) in all four
script builders, isolating GitHub CLI identity per claude-multi account.
Spec: `docs/specs/2026-07-02/spec-isolate-gh-config-per-account.md` (AC1-AC6).

**QA review done 2026-07-02** (branch `feat/isolate-gh-config-per-account`,
commits 1457106 impl+tests, 190f7a7 docs, cdd1867 QA test additions):
- AC1/AC2/AC5 (automated): covered. Added 4 missing PowerShell-ordering
  tests (build/login/logout/relogin) — POSIX variants had dedicated ordering
  assertions, PowerShell variants only checked substring presence before this.
- AC3/AC6 (manual/docs): covered — SMOKE-CHECKLIST.md and README/CHANGELOG
  updated in the same PR.
- AC4 (inspection): confirmed via grep — only `launcher.rs` references `gh`;
  no code path in `inherit.rs`/`commands.rs`/others touches `<dir>/gh` or the
  global `~/.config/gh`.
- Full suite: `cargo test` 72 passed, `cargo clippy --all-targets -- -D
  warnings` clean, `cargo fmt --all -- --check` clean.

**Known low-risk quirks, not treated as bugs (pre-existing pattern, no fix
requested):**
- Trailing slash in `config_dir` (e.g. user-entered `~/.claude-x/`) produces
  a doubled separator (`.../.claude-x//gh` POSIX, `...\.claude-x\\gh` PS).
  Cosmetic only — POSIX collapses repeated `/`; not verified against real
  `gh` on Windows for literal `\\`. `config_dir` is never normalized
  anywhere in the codebase (same as pre-existing `CLAUDE_CONFIG_DIR`), so
  this isn't new risk introduced by this feature.
- Empty `config_dir` (only reachable via hand-edited config.json, not the
  Preferences UI which always prepends `~/.claude-`) makes `GH_CONFIG_DIR`
  resolve to the concrete absolute path `/gh` (POSIX), a shared location
  across any misconfigured accounts — worse than the ambiguous empty
  `CLAUDE_CONFIG_DIR=''` it inherits the risk from. Not defended against;
  flagged as a finding, not fixed (out of scope, no validation layer exists
  for `config_dir` anywhere in `config.rs`).

See [[feedback-launcher-test-substring-gotcha]] for a test-writing pitfall
hit while closing the PowerShell ordering gap, and
[[project-claude-multi-overview]] for general launcher.rs review checklist.
