//! Subscription-OAuth backends.
//!
//! v0 ships only the trait surface and policy enum (in the parent `auth`
//! module). Concrete backends — Copilot, Codex, Gemini, Claude Code — land as
//! their own commits with research notes per `docs/research/models/oauth.md`.
//!
//! Module ordering reflects intent:
//! 1. `copilot`  — PRIMARY, contractually clean, GitHub-issued OAuth
//! 2. `codex`    — GreyArea, tolerated in practice
//! 3. `gemini`   — Allowed, free-tier capable
//! 4. `claude_code` — EnforcedBlock since 2026-04-04, off by default
