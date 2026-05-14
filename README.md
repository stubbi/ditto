# Ditto

An agent harness with memory coherence, OS-enforced sandboxing, multi-tenant by design, and eval-as-CI.

**Status:** v0.0.1. Memory architecture is committed (see [`docs/architecture/memory.md`](docs/architecture/memory.md) v2). First Rust crates are scaffolded:

```
crates/
  ditto-core/             types, canonical JSON, content addressing, Ed25519 signing
  ditto-memory/           MemoryController + Storage trait + bi-temporal NC-graph
  ditto-models/           Provider/CapabilitySet traits, typed streaming events, ToolRegistry + JIT projection, CallCost; first adapter: OpenRouter
  ditto-mcp/              MCP server transport (rmcp) — 8 memory tools
  ditto-render/           NC-doc renderer (bi-temporal Markdown pages from the graph)
  ditto-storage-postgres/ Postgres backend (sqlx + tsvector BM25 + nc_node + nc_edge)
  ditto-cli/              `ditto migrate / write / search / keygen / render / serve`
migrations/
  initial_schema.sql      episodic + receipt tables
eval/
  ditto-eval Python package — see eval/README.md
```

149 tests green (canonical JSON determinism, content addressing, Ed25519 sign/verify, hash chain, signed receipts, idempotent writes, in-memory search, bi-temporal edge supersession, time-travel queries, retroactive invalidation, deterministic NC-doc render, idempotent re-render, historical-facts section, manifest content hashing, removal cascade). Cross-language interop check: `EventId` computed in Rust matches `content_address` in the Python eval harness for the same payload bit-for-bit.

```bash
cargo build && cargo test                 # 25/25 pass
cargo run --bin ditto -- keygen           # print 32-byte Ed25519 install secret (hex)
cargo run --bin ditto -- write \
  --tenant <uuid> --scope <uuid> \
  --source test --payload '{"content":"hello"}'
```

In-memory backend is the default when no `--database-url` is set; Postgres backend works once `ditto migrate` has run against a database.

## Why Ditto exists

Today's agent harnesses (hermes-agent, openclaw, openhuman, gbrain on top) ship overlapping unfinished work in five areas:

1. **Memory is contested.** Hermes flattens it into `MEMORY.md`. Openclaw runs three competing memory subsystems. Openhuman's Memory Tree panics on UTF-8 boundaries. Every project that bolts a memory layer onto an existing harness inherits a concurrency bug class (single-writer hazards, HNSW corruption, hook races).
2. **"Private" is marketing, not enforcement.** Openhuman's sandbox is logical-only on macOS and Windows. Openclaw has 15+ security issues open and quiet for months. Hermes leaves `.env` world-readable.
3. **API keys are the onboarding wall.** The most-asked feature across gbrain (#94, #334, #679, #777) and openhuman (#1554, #1673) is subscription-native auth — Claude Code, Codex, ChatGPT OAuth — not raw OpenAI/Anthropic keys.
4. **Multi-tenant doesn't exist.** Openclaw's RBAC RFC (#8081, 28 reactions) was closed as not planned. Hermes is single-user by design. Gbrain's source isolation leaks (#428, #705, #891).
5. **Eval is vibes.** Only gbrain ships honest benchmarks. Mempalace's headline number measures retrieval recall, not answer quality (#27, 335 reactions document the gap).

Ditto's bet: a harness that owns memory coherence, runs with real OS sandboxing, ships multi-tenant from line one, and treats eval as CI is structurally Pareto-better than any incumbent on the axes power users actually pay for.

## Design pillars

1. **Memory-coherence as a harness boundary.** Ditto is the single writer to memory. Seven typed slots (working / episodic-index / blob-store / NC-graph / NC-doc / procedural / reflective) with bi-temporal validity, SCITT-compliant signed receipts, hippocampal-indexed episodic + content-addressed blob storage, surprise-gated writes, reconsolidation labile windows, three-cadence replay (awake ripple / dream cycle / long sleep), and an RL-trained memory operations policy. See [`docs/architecture/memory.md`](docs/architecture/memory.md) for the v2 commitment and [`docs/research/memory.md`](docs/research/memory.md) for the synthesis across landscape, arxiv frontier, neuroscience, production, and trending research.
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
    memory.md            v2: seven slots, controller, three-cadence replay, eval
    models.md            Provider trait, SubscriptionBackend, JIT tool projection
  research/
    memory.md            synthesis entry point across all five vectors below
    memory/
      landscape.md       Round 1: incumbent landscape (Mem0/Zep/Letta/MemPalace/gbrain)
      arxiv.md           frontier research papers, 2025-Q4 to 2026-Q2 (~80 citations)
      biology.md         neuroscience grounding (CLS, hippocampal indexing, etc.)
      production.md      forensic survey of ~30 deployed AI products
      trending.md        OSS velocity, practitioner pains/asks, surprise findings
    models.md            synthesis entry point for the model-routing research
    models/
      landscape.md       LiteLLM, Vercel AI SDK, Rust ecosystem, provider divergence
      oauth.md           every subscription-OAuth flow in 2026, TOS analysis
      community.md       practitioner pain catalog (HN, X, Reddit, GitHub issues)
eval/
  ditto-eval Python package — benchmark harness for memory backends
  (Mem0, Zep, Mastra, MemPalace, gbrain, Ditto). Eval-first: we measure
  incumbents on the same fixtures we'll measure ourselves on, before
  writing any memory-engine code. See eval/README.md.
```

## Migration

Ditto is not a hosted version of hermes-agent or openclaw. It imports their state once via `ditto import` and runs natively. See [`docs/architecture/importer.md`](docs/architecture/importer.md).

## License

Not yet chosen. Open issue.
