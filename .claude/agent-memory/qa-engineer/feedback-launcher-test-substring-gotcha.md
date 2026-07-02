---
name: feedback-launcher-test-substring-gotcha
description: Naive substring search in launcher.rs script tests can false-positive/false-negative because account config dirs contain the literal word "claude"
metadata:
  type: feedback
---

When writing ordering assertions in `src-tauri/src/launcher.rs` tests (e.g.
"GH_CONFIG_DIR must be set before the `claude` invocation runs"), do not use
`s.find("claude")` or similar bare substring search to locate the actual
command-invocation line.

**Why:** Test fixtures conventionally use config dirs like
`.claude-personal` / `.claude-dino` (matching real-world `~/.claude-<suffix>`
naming). That literal string contains "claude", so a naive `s.find("claude")`
matches inside the earlier `CLAUDE_CONFIG_DIR = '...claude-personal...'` line
instead of the actual trailing `claude` command line — producing a test that
fails even though the implementation is correct (self-inflicted false
positive, caught while adding PowerShell-ordering tests on
2026-07-02, see [[project-gh-isolation-feature]]).

**How to apply:** Anchor on an unambiguous, delimited pattern instead:
- POSIX: search for `"exec claude"` or `"cd '"` (existing tests already do
  this correctly).
- PowerShell bare invocation (no `exec`/`cd` prefix, e.g. `build_login_script`):
  use `s.rfind("\nclaude\n")` to find the standalone invocation line, not
  `s.find("claude")`.
This applies to any future launcher.rs test where the interpolated
`config_dir` value might coincidentally contain a substring you're also
searching for as a structural marker.
