# Spec: Block launches for logged-out accounts + enforce a single app instance

| Field | Value |
|-------|-------|
| **Date** | 2026-08-18 |
| **Author** | Lukeneo12 |
| **Status** | Implemented (pending manual smoke §17/§18) |
| **Type** | Fix (two independent fixes, one PR) |
| **Related** | `docs/specs/2026-07-11/…` (n/a), PR #11 (expired-session detection) |

---

## 1. Context / Problem

### 1a. Logged-out accounts can still launch projects

`session.rs` derives an account's state from two sources: the recorded email in
`<config_dir>/.claude.json` (`oauthAccount.emailAddress`) and the OAuth tokens
(`<config_dir>/.credentials.json`, or the macOS Keychain entry
`Claude Code-credentials-<hash>`). PR #11 made the verdict deliberately
conservative: only *positive* evidence of expiry blocks an account, and any
missing/unreadable credentials collapse to `Unknown` → `LoggedIn`.

That collapse hides the plain logout case. When the tokens are gone (Keychain
item deleted by `claude auth logout` / `/logout`, or no `.credentials.json`)
but `.claude.json` still carries `oauthAccount`, the tray shows the account as
`✓ <email>` with all projects launchable, and `launch_session` /
`open_session` happily open a terminal that immediately asks the user to log
in. Verified against the CLI itself: `claude auth status` reports
`loggedIn: false` for a config dir that has `oauthAccount` but no credentials
— i.e. Claude Code decides login state from the credentials alone.

What the user actually observed (2026-08-18): without ever logging out, a
launched session asked for login while the tray still listed the projects.
Both `claude auth logout` and an in-session `/logout` were later confirmed to
strip `oauthAccount` too, so the "email left behind" state is **not** produced
by a normal logout — it is produced when the tokens disappear by another route
(most plausibly Claude Code dropping them after a failed refresh; also manual
Keychain/file deletion). That is the case this spec makes detectable. The
sibling case — tokens still stored locally but rejected server-side — is
explicitly out of scope (see Non-goals) and can't be told apart offline; if it
recurs, capture the credential *shape* (keys + expiries, no secrets) before
re-logging in to decide whether it needs its own follow-up.

Two gaps, then:
- **Detection**: "credentials absent" is indistinguishable from "credentials
  unreadable" (e.g. a denied Keychain prompt), so it inherits the
  never-lock-out fallback.
- **Gate**: `ensure_session_usable` only refuses `Expired`; a `LoggedOut`
  account passes, so any stale menu or direct `invoke` still launches.

### 1b. A second launch spawns a second app

There is no single-instance guard. Launching claude-multi while it is already
running starts a second process with its own tray icon; both rebuild the tray
from the same config and both react to menu events, which is confusing and can
double-launch sessions.

## 2. Goals / Non-goals

### Goals
- G1: An account whose credentials are **absent** is reported `LoggedOut`,
  regardless of whether `.claude.json` still records an email; the tray then
  offers only `Login…` for it, preceded by a disabled `○ <email> — logged out`
  line when the email is still recorded (so the user knows which account it
  was; confirmed with the user on 2026-08-18).
- G2: `launch_session` and `open_session` refuse `LoggedOut` accounts with a
  clear message pointing at `Login…` (backstop for stale menus / direct calls).
- G3: A denied or otherwise unreadable Keychain read stays `Unknown` →
  `LoggedIn` (no regression of the never-lock-out guarantee from PR #11).
- G4: The verdict cache does not make a fresh login look logged-out for 30s:
  `Absent` verdicts are not cached (an absent Keychain item returns instantly
  and never prompts), and a Login/Logout/Re-login action from the tray drops
  the account's cache entry **and** opens a 120s grace window during which the
  dir is never re-cached — the credentials change only when the user finishes
  in the terminal, so a hover in between must not pin the pre-action state for
  another TTL (review finding on PR #13).
- G5: Only one instance of the app runs; a second launch focuses the running
  instance's Preferences window and exits.
- G6: Pure decision logic covered by `test_should_X_when_Y` unit tests;
  `cargo clippy --all-targets -- -D warnings` stays clean.

### Non-goals
- Detecting server-side revocation (tokens present locally but rejected by
  the API). The only real validation of a refresh token is *using* it, and
  Anthropic rotates refresh tokens — a probe from the app would clobber the
  token Claude Code holds. Out of scope — the existing expiry logic plus this
  fix cover everything observable offline.
- Shelling out to `claude auth status` from the app. It is authoritative but
  the app process lacks the user's `PATH` (see project invariants), and it
  writes into the config dir (`backups/`, lock) as a side effect.
- Any other tray-layout change for `LoggedOut` beyond the status line above.
- Handling `--args` / deep links passed to the second instance; it just
  focuses the first.

## 3. Acceptance Criteria

- [ ] AC1: Given `.claude.json` has `oauthAccount.emailAddress` and no
      `.credentials.json` exists and (macOS) the Keychain item is not found,
      when the tray menu is built, then the account submenu shows a disabled
      `○ <email> — logged out` line and `Login…`, nothing else (state
      `LoggedOut { last_email: Some(..) }`); an account with no recorded email
      shows only `Login…` (`last_email: None`).
- [ ] AC2: Given the same state, when `launch_session` or `open_session` is
      invoked for that account (tray or `invoke`), then it returns
      `Err` mentioning the account label and `Login…`, and no terminal is
      spawned.
- [ ] AC3: Given credentials exist but cannot be read (macOS `security` exits
      non-zero for a reason other than "item not found", or the file is
      unreadable / unparseable), then the account stays `LoggedIn` and
      launches are allowed (unchanged behavior).
- [ ] AC4: Given a valid, non-expired credential blob, the account is
      `LoggedIn` (unchanged); given positive expiry evidence, `Expired`
      (unchanged).
- [ ] AC5: `Absent` verdicts are never stored in the verdict cache; triggering
      Login / Logout / Re-login from the tray drops the cached verdict for that
      account's config dir and, for the next 120s, every menu build re-reads
      that dir (a state written mid-window is visible on the next hover).
- [ ] AC6: With the app already running, starting it again does not create a
      second tray icon; the existing instance's Preferences window is shown
      and focused, and the second process exits.
- [ ] AC7: New unit tests for the pure classification (`security` exit
      status/stderr → `Absent` vs `Unreadable`, and verdict → status mapping)
      pass; `cargo test` and clippy `-D warnings` are green.

## 4. Approach

### 4a. `session.rs`

1. Replace `read_credentials(&Path) -> Option<String>` with an explicit
   three-way source result:
   ```rust
   enum CredentialSource { Found(String), Absent, Unreadable }
   ```
   - File path: `read_to_string` `Ok` → `Found`; `ErrorKind::NotFound` →
     fall through to the Keychain on macOS / `Absent` elsewhere; any other
     error → `Unreadable`.
   - macOS Keychain: success → `Found`; classify failures with a **pure**
     `classify_security_failure(exit_code: Option<i32>, stderr: &str)`:
     exit code `44` or stderr containing `"could not be found"` → `Absent`;
     anything else (user denied the prompt, interaction not allowed, etc.) →
     `Unreadable`. (Implemented as `security_item_not_found(...) -> bool` so
     the source layer doesn't return a verdict-layer enum.)
2. Add `CredentialVerdict::Absent`. `cached_verdict` maps `Absent` →
   `Absent` (bypassing the cache write), `Unreadable` → `Unknown`,
   `Found(json)` → `evaluate_credentials`.
3. `account_session_status`: `Absent` → `LoggedOut { last_email: Some(email) }`
   even when an email is recorded (`SessionStatus::LoggedOut` gains an
   `Option<String>` so the tray can name the account). Add a pure helper
   `status_from(email: Option<String>, verdict: CredentialVerdict) -> SessionStatus`
   so the mapping is unit-testable without I/O.
4. `ensure_session_usable`: `LoggedOut` → `Err("'<label>' is not logged in. Use “Login…” in this account's tray menu first.")`.
5. `pub fn invalidate_verdict(config_dir: &Path)`: drops the cache entry and
   records `now + AUTH_GRACE (120s)` in a `no_cache_until` map; `cached_verdict`
   neither reads nor writes the cache for a dir inside its grace window. Called
   from `commands::run_account_action` for `Login | Logout | Relogin`.

Trade-off: `Absent` on macOS relies on the `security` CLI's not-found
signal. Matching both the exit code (44) and the message keeps it robust to
either changing; if both change, the failure mode is `Unreadable` →
`LoggedIn`, i.e. today's behavior, never a lock-out.

### 4b. Single instance (`lib.rs`, `Cargo.toml`)

Add `tauri-plugin-single-instance = "2"` and register it **first** in the
builder (required by the plugin), with a callback that shows + focuses the
`main` window:
```rust
.plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
    if let Some(w) = app.get_webview_window("main") { let _ = w.show(); let _ = w.set_focus(); }
}))
```
The callback reuses `tray::show_preferences`, shared with the tray's
"Preferences…" item. No frontend or capability changes needed (the plugin has
no JS API).

Alternative discarded: a lock file / PID check under the app-config dir —
more code, no cross-instance signalling, stale-lock handling.

## 5. Risks / Rollback

- **Risk**: on macOS a `security` failure that is really "not found" but with
  a different exit code/message → still `LoggedIn` (status quo, not worse).
- **Risk**: an account authenticated by API key (no OAuth) — already
  `LoggedOut` today (no `oauthAccount`), no change.
- **Risk**: single-instance plugin behavior on macOS when the app is started
  from a non-bundle binary during `tauri dev` — the plugin supports all three
  desktop OSes; if it misbehaves in dev, it's a one-line plugin removal.
- **Rollback**: revert the PR; no config or on-disk format changes.
