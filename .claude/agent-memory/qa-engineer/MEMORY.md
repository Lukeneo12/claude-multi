# Memory Index

- [claude-multi project overview](project-claude-multi-overview.md) — Tauri v2 tray app, per-account isolated config dirs, pure-fn launcher.rs is the TDD-heavy module
- [launcher.rs test gotcha: "claude" substring in dir names](feedback-launcher-test-substring-gotcha.md) — account dirs like `.claude-personal` contain literal "claude", breaks naive `s.find("claude")` assertions
- [gh isolation feature (2026-07-02)](project-gh-isolation-feature.md) — spec + AC coverage status for per-account GH_CONFIG_DIR isolation
