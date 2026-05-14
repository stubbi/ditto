# Ditto

An agent harness with memory coherence, OS-enforced sandboxing, multi-tenant by design, and eval-as-CI.

**Status:** pre-alpha. This repo currently holds architecture sketches, not code. The codebase begins after the architecture stabilizes.

## Why Ditto exists

Today's agent harnesses (hermes-agent, openclaw, openhuman, gbrain on top) ship overlapping unfinished work in five areas:

1. **Memory is contested.** Hermes flattens it into `MEMORY.md`. Openclaw runs three competing memory subsystems. Openhuman's Memory Tree panics on UTF-8 boundaries. Every project that bolts a memory layer onto an existing harness inherits a concurrency bug class (single-writer hazards, HNSW corruption, hook races).
2. **"Private" is marketing, not enforcement.** Openhuman's sandbox is logical-only on macOS and Windows. Openclaw has 15+ security issues open and quiet for months. Hermes leaves `.env` world-readable.
3. **API keys are the onboarding wall.** The most-asked feature across gbrain (#94, #334, #679, #777) and openhuman (#1554, #1673) is subscription-native auth — Claude Code, Codex, ChatGPT OAuth — not raw OpenAI/Anthropic keys.
4. **Multi-tenant doesn't exist.** Openclaw's RBAC RFC (#8081, 28 reactions) was closed as not planned. Hermes is single-user by design. Gbrain's source isolation leaks (#428, #705, #891).
5. **Eval is vibes.** Only gbrain ships honest benchmarks. Mempalace's headline number measures retrieval recall, not answer quality (#27, 335 reactions document the gap).

Ditto's bet: a harness that owns memory coherence, runs with real OS sandboxing, ships multi-tenant from line one, and treats eval as CI is structurally Pareto-better than any incumbent on the axes power users actually pay for.

## Design pillars

1. **Memory-coherence as a harness boundary.** Ditto is the single writer to memory. Typed slots (working / episodic / semantic / procedural) with explicit lifecycle, eviction, and provenance. See [`docs/research/memory.md`](docs/research/memory.md) (in-flight) and [`docs/architecture/memory.md`](docs/architecture/memory.md) (forthcoming).
2. **Secure-by-default execution.** macOS Seatbelt, Windows AppContainer, Linux Landlock as enforced defaults. Credential brokering — the agent process never holds raw API keys. Per-tool egress policy.
3. **Subscription-native model routing.** Claude Code, Codex, ChatGPT OAuth as first-class. BYOK via OpenRouter/LiteLLM as fallback. JIT tool-schema projection — no 14k-token preamble per turn.
4. **Multi-tenant from line one.** Org / tenant / workspace / agent hierarchy. Per-tenant secret vault. Postgres RLS. Audit log. See [`docs/architecture/multi-tenant.md`](docs/architecture/multi-tenant.md).
5. **Eval-as-CI.** Agentic, retrieval, and routing benchmarks against pluggable backends. Held-out splits, per-question fixtures, regression gates.

## Repo layout

```
docs/
  architecture/
    multi-tenant.md      data model, tenancy hierarchy, auth, secrets, audit
    importer.md          one-shot import from hermes-agent and openclaw
    memory.md            (forthcoming) Ditto's memory architecture
  research/
    memory.md            (in-flight) state of the art in agent memory systems
```

## Migration

Ditto is not a hosted version of hermes-agent or openclaw. It imports their state once via `ditto import` and runs natively. See [`docs/architecture/importer.md`](docs/architecture/importer.md).

## License

Not yet chosen. Open issue.
