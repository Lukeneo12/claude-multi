# Spec: CLI Execution Layer (`cms`)

| Field | Value |
|-------|-------|
| **Date** | 2026-09-14 |
| **Author** | Lukeneo12 |
| **Status** | Draft |
| **Type** | Feature |
| **Related PRD** | N/A |

---

## 1. Context / Problem

claude-multi-session launches per-account Claude Code sessions exclusively through the
system tray, which spawns a **new** terminal window via a terminal adapter. This breaks
down inside IDEs with integrated terminals (e.g. Orca): the user is already sitting in
a terminal, in the project directory, and wants the session to run **inline** in that
terminal under a specific account — but the only entry point opens a separate window
through a configured adapter.

Today the workaround is manually typing
`CLAUDE_CONFIG_DIR=~/.claude-<suffix> claude`, which skips everything the tray launch
guarantees: per-account `GH_CONFIG_DIR` isolation, the session-usability gate
(`LoggedOut`/`Expired` detection), and the inherit/seeding of user-level `~/.claude`
resources (agents, commands, skills, output-styles, settings.json) into the account
dir. Accounts launched this way silently diverge from tray-launched ones.

Desired state: a `cms` command available in any terminal that resolves an account from
the app's own config and launches `claude` inline with full parity with the tray flow.

## 2. Goals / Non-goals

### Goals
- A `cms` CLI binary, built from the existing `src-tauri` crate, installable to the
  user's `PATH`.
- `cms <account> [extra claude args…]` launches `claude` **in the current terminal**
  (no adapter, no new window), using the **current working directory** as the project.
- Account matching is case-insensitive against both `id` and `label`, accepting a
  unique prefix (`dino` → label `Dinocloud`); ambiguous or unmatched input fails with
  a clear error listing available accounts.
- Full tray parity before launch: `ensure_session_usable` gate, then inherit + seed
  (best-effort, same semantics as the tray flow).
- `cms --list` prints configured accounts (id, label, logged-in email when available).
- Arguments after the account name are passed through verbatim to `claude`
  (e.g. `cms dino --resume`).
- Standalone config-path resolution (no Tauri `AppHandle`) that resolves the **same**
  `config.json` the app reads/writes.

### Non-goals
- No `login`/`logout`/`relogin` subcommands — auth management stays in the tray. The
  gate's error message points the user at the tray.
- No project-by-name launching (`cms <account> <project>`) and no interactive picker —
  the cwd is the project.
- No workspace refactor (extracting a `claude-multi-core` crate). The bin target links
  the existing crate as-is; the refactor remains a future option if it hurts.
- No changes to tray behavior, Preferences UI, or config schema.
- No shell completions or packaging (brew, etc.) in this iteration.

## 3. Acceptance Criteria

- [ ] AC1: Given account label `Dinocloud` (id `a1`), `cms dino`, `cms a1`, and
  `cms DINOCLOUD` all resolve to that account; `cms x` (no match) and an ambiguous
  prefix both exit non-zero with an error listing `id (label)` for every configured
  account.
- [ ] AC2: `cms <account>` run from directory `D` replaces the CLI process with
  `claude` (Unix `exec`; Windows spawn + wait, propagating the exit code) with
  `CLAUDE_CONFIG_DIR=<expanded account config_dir>` and
  `GH_CONFIG_DIR=<config_dir>/gh` set, and cwd `D`. No terminal adapter is invoked
  and no temp script is written.
- [ ] AC3: Given an account whose session evaluates to `LoggedOut` or `Expired`,
  `cms <account>` exits non-zero **before** launching `claude`, with a message naming
  the account and pointing at the tray's Log in / Re-login action.
- [ ] AC4: Before exec, `cms` performs the same inherit + seeding as the tray launch
  (links for `agents`/`commands`/`skills`/`output-styles` honoring per-account
  `inherit_overrides`, one-shot seed of `settings.json`); failures there are
  best-effort exactly as in the tray flow and never write inside `~/.claude`.
  Subdirs whose conflict is still undecided are skipped for that launch with a
  stderr warning pointing at the tray — the CLI never opens a GUI prompt and
  never writes `inherit_overrides` to `config.json`.
- [ ] AC5: `cms --list` prints every configured account as `id  label  [email|not
  logged in]` and exits 0; `cms` with no arguments prints usage + the account list
  and exits non-zero.
- [ ] AC6: The standalone config path equals Tauri's `app_config_dir` for identifier
  `com.lucasdonadio.claude-multi` on macOS (`~/Library/Application
  Support/com.lucasdonadio.claude-multi/config.json`), with the analogous
  `dirs::config_dir()`-based path on Linux/Windows. If the file is missing, `cms`
  reports that the app has not been configured yet (it does not create a default
  config).
- [ ] AC7: `cms dino --resume` invokes `claude --resume` (trailing args pass through
  in order, untouched).
- [ ] AC8: `cargo test` covers the new pure logic (account matching incl. ambiguity,
  arg splitting/pass-through, config-path resolution, launch-plan construction) with
  `test_should_X_when_Y` naming; `cargo clippy --all-targets -- -D warnings` stays
  clean; `npm run build` unaffected.
- [ ] AC9: `npm run install-cli` (or documented equivalent) builds the release bin and
  symlinks it into `~/.local/bin/cms`.

## 4. Approach

New `[[bin]] name = "cms"` target in `src-tauri` (the crate/app binary name
`claude-multi` is taken by the Tauri app). The bin is a thin `main` over a new
Tauri-free module with the testable logic:

- `src-tauri/src/cli.rs` — pure logic: `match_account(&[Account], query) ->
  Result<&Account, MatchError>` (exact id/label match wins over prefix; ambiguity is
  an error carrying candidates), `parse_args` (account query, `--list`, trailing
  claude args), and launch-plan assembly (expanded config dir + env pairs, reusing
  `PER_ACCOUNT_ENV_VARS` semantics from `launcher.rs`).
- `src-tauri/src/paths.rs` — add `standalone_config_file_path() -> Option<PathBuf>`
  using the `dirs` crate + the hardcoded bundle identifier, mirroring Tauri's
  `app_config_dir` per OS.
- `src-tauri/src/bin/cms.rs` — edge I/O only: load config, resolve account, call
  `session::ensure_session_usable`, run inherit + seed (reusing the same sequence as
  `commands::ensure_account_inherits`, minus the `AppHandle`-based path lookup),
  set env via `std::process::Command` (no shell, no script, no escaping needed) and
  `exec` `claude` (`std::os::unix::process::CommandExt::exec`; Windows:
  `status()` + `exit(code)`).
- `src-tauri/src/lib.rs` — expose the needed modules as `pub` so the bin can link
  `claude_multi_lib`.
- `commands.rs` — refactor `ensure_account_inherits` so its account-level body
  (inherit + seed given a `&Config` + account id) is shared with the CLI instead of
  duplicated.
- `package.json` — `install-cli` script: `cargo build --release --bin cms` +
  `ln -sf` into `~/.local/bin`.
- Docs: `CHANGELOG.md` entry, `docs/SMOKE-CHECKLIST.md` section for `cms`, README
  mention.

### Key decisions
- **Bin target in the existing crate (approach A):** zero logic duplication and
  guaranteed parity because the CLI calls the very same `session`/`inherit`/`config`
  code the tray uses. Tradeoff: the bin links the full crate including Tauri deps —
  larger binary, slower cold build — accepted because nothing Tauri is initialized at
  runtime and install is a one-time build.
- **No script generation:** env is set through the `Command` API directly, so the
  shell-escaping invariant doesn't apply (nothing is interpolated into shell text).
  `claude` resolves via the terminal's own `PATH`, satisfying the GUI-PATH invariant
  trivially.
- **Missing config ≠ default config:** unlike the app, `cms` refuses to run without an
  existing `config.json` — silently inventing a default `personal` account from a CLI
  would create `~/.claude-personal` state the user never configured.
- **`cms` as the name:** short and typeable; `claude-multi` collides with the app
  binary name.
- **No GUI prompt, no config writes from the CLI:** the tray flow prompts (dialog)
  when an inherited subdir has an undecided conflict and persists the decision. The
  CLI skips such subdirs for that launch and warns on stderr instead — avoids
  concurrent `config.json` writes while the app is running and keeps the CLI
  non-interactive. The shared inherit logic is extracted so both callers use
  identical semantics and only the surfacing differs.

### Alternatives considered
- **Option B — workspace refactor into `claude-multi-core`:** cleaner dependency
  graph, lighter CLI binary — rejected for now as a large mechanical refactor with no
  functional gain; approach A keeps it available later.
- **Option C — argv handling in the Tauri app itself:** rejected: conflicts with the
  single-instance plugin (second invocation focuses Preferences and exits) and the
  binary lives inside the `.app` bundle.
- **Option D — app-generated shell function/script:** rejected: duplicates launch
  logic outside Rust, no session gate, no inherit/seed, drifts from the tray.

## 5. Risks / Rollback

### Risks
| Risk | Probability | Impact | Mitigation |
|------|-------------|--------|------------|
| Standalone config path diverges from Tauri's `app_config_dir` on some OS | Low | High (CLI silently reads a different/empty config) | Unit-test the macOS path exactly; document the identifier in one shared constant; `cms --list` makes a wrong path immediately visible (empty account list + path printed in the error) |
| Keychain prompt on session check when run from CLI context | Low | Med | `session.rs` already treats *unreadable* credentials as `LoggedIn` (never locks out on a denied prompt); same behavior applies unchanged |
| `dirs` crate added as a dependency | Low | Low | Tiny, widely used, no transitive weight compared to existing Tauri tree |
| Refactor of `ensure_account_inherits` breaks tray launch | Low | High | Behavior-preserving extraction covered by existing tests + smoke checklist run |

### Rollback plan
Purely additive feature: revert the PR (or delete the `[[bin]]` target, `cli.rs`,
`bin/cms.rs`, and the `install-cli` script) and remove `~/.local/bin/cms`. No config
schema, tray, or account-dir state changes to undo.

## 6. Open questions
- [ ] None — design approved in brainstorming session 2026-09-14.

---

*Spec generated with `/spec` skill. Update this file if the approach changes during implementation.*
