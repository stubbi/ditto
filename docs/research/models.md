# Models research — synthesis

*Compiled 2026-05-14 across three deep-research vectors: library landscape, subscription-OAuth flows, and practitioner discourse.*

This is the entry point. The research lives in three sub-documents under [`models/`](./models/):

- [`landscape.md`](./models/landscape.md) — LiteLLM, Vercel AI SDK, Portkey, Helicone, OpenRouter SDK, OpenAI Agents SDK, the Rust ecosystem (rig, genai, async-openai, swiftide), provider divergence map per feature, JIT tool-schema projection techniques, ~80 citations.
- [`oauth.md`](./models/oauth.md) — every subscription-OAuth flow that exists in May 2026, the technical detail, the TOS reality, what got banned and when.
- [`community.md`](./models/community.md) — practitioner pain catalog from HN, X, Reddit, dev.to, GitHub issues. Quoted, sourced.

What follows is the synthesis: cross-cutting findings, the strategic reframe of Bet 3 in light of the OAuth reality, and the architectural commitments these reports drive. The detailed architecture lives in [`../architecture/models.md`](../architecture/models.md).

---

## The single most important finding

**Anthropic banned third-party Claude Code OAuth on 2026-04-04.** Server-side blocks deployed Feb–March 2026; full enforcement April 4, 3PM ET. OpenCode received a legal request and removed support. Per Anthropic's policy (verbatim): *"Use of OAuth tokens obtained via Claude Free, Pro, or Max accounts in any other product, tool, or service — including the Agent SDK — is not permitted."*

This invalidates the obvious version of Bet 3 ("let users plug their Claude subscription into Ditto"). It does **not** invalidate Bet 3 itself — the user demand it solved ("I don't want a second billing relationship") is solved by **any** subscription that grants multi-model access, not Claude specifically.

The actual May 2026 subscription-multi-model paths, ranked:

| Path | Status | Models accessible | Recommendation |
|---|---|---|---|
| **GitHub Copilot OAuth** | Contractually clean. Used by continue.dev, aider, opencode, LiteLLM. | Claude Opus 4.7, GPT-5.5, Gemini 2.5 Pro, o-series — all in one $10–39/mo auth | **Primary wedge.** Lead onboarding with this. |
| **OpenAI Codex OAuth** | Documented, gray-but-tolerated. OpenAI partnered with Cline. `openai/codex#8338` ToS discussion open. | OpenAI models | Secondary subscription path. |
| **Gemini Code Assist OAuth** | Free tier (60 rpm / 1000 rpd, no credit card). Personal Google account = zero friction. | Gemini models | Zero-friction onboarding ("try Ditto in 30s"). |
| **Claude Code OAuth** | **Banned. Do not ship a "Sign in with Claude" wizard.** | — | Document the `CLAUDE_CODE_OAUTH_TOKEN` env-var escape (Anthropic supports it for CI). User-provided tokens only. |
| **OpenRouter BYOK** | API-key, pay-per-token. Vast catalog including Claude via Anthropic's relationship. | 200+ models | **Default BYOK catalog.** Multi-model without the Claude-OAuth legal risk. |
| **Direct provider BYOK** | Anthropic, OpenAI, Bedrock, Vertex, Cerebras, Groq, etc. | Native features | Power-user path for cache_control, batch APIs, structured outputs. |

The Ditto pitch in one sentence: **"Bring your Copilot, your Codex, your Gemini, or any BYOK — Ditto routes across all of them with one configuration."**

---

## Other cross-cutting findings

### Eight findings the three reports agree on

1. **JIT tool-schema projection is non-negotiable.** Hermes-agent #4379 measured 13,935 tokens (73%) of fixed overhead per call. MCP SEP-1576 documents 17.6K tokens for GitHub's MCP server alone — multi-MCP setups hit 30K+ tokens *before any user content*. Anthropic's Nov 2025 "code execution with MCP" pattern reports **98.7% reduction** (150k → 2k tokens) by exposing MCP servers as importable code modules. Claude Code v2.1.7 ships MCP Tool Search that activates when tools exceed 10% of context. No incumbent does this cleanly across providers.

2. **LiteLLM is the cautionary tale, not the template.** March 24 2026 supply-chain attack (versions 1.82.7/1.82.8 published with credential-stealing `.pth` payload via Trivy CI/CD compromise). 800+ open issues. Performance cliff at 500 RPS. Streaming bugs that drop ~90% of tool-argument deltas (#20711), collapse parallel tool calls to index=0 (#21331), reject Anthropic beta headers on Vertex (#14293). YAML routing config is a *de facto* standard but produces silent failures. Practitioners are explicitly migrating away.

3. **Vercel AI SDK 6's streaming ontology is the right floor.** `text-delta` / `tool-call-start` / `tool-input-delta` / `tool-input-end` / `tool-result` / `reasoning-delta` / `finish` events with usage payload. Anything coarser (raw `Stream<String>` or "events flattened to text") loses information practitioners need. Above this floor, Ditto must preserve cache-read/write/reasoning token counts as distinct lines and the `provider_executed` flag for gateway-tools.

4. **The Rust ecosystem has no LiteLLM-quality crate.** `genai` v0.5 has the most breadth (17 providers including a Copilot adapter) — closest in capability. `rig` v0.37 is most ergonomic — but its author warns of frequent breaking changes. Both flatten provider-specific features (cache_control, reasoning effort, extended thinking) through opaque options. Pragmatic choice: depend on `genai` for SSE transports + the existing Copilot adapter, own everything else.

5. **`models.dev` is the de-facto shared catalog.** opencode, cline, aider, continue.dev all converged on it. `providerId/modelId` syntax is what users already know. Ditto should consume it as a data source (with local override), not maintain its own.

6. **Provider divergence does not flatten cleanly.** Anthropic explicit `cache_control` ≠ OpenAI implicit auto-cache ≠ Vertex (Anthropic-shape but rejects the same beta header). Anthropic `tool_use` blocks interleaved with text ≠ OpenAI `tool_calls` array streamed by index ≠ Gemini `function_call` parts. Reasoning surfaces (Anthropic extended-thinking blocks, OpenAI o1 hidden, DeepSeek `<think>` tags) need their own typed representation. The abstraction must let typed extensions ride through, never silently drop them.

7. **OpenRouter quality drift is real.** Defaults to mixed quantization providers (Venice, NextBit, Mancer, DeepInfra). Community complaints in roo-cline #11325 (CJK encoding broken on FP4/Int4), qwen-code #348 ("avoid quantized models on OpenRouter"). Their own "Exacto" launch concedes "providers tweak inference stacks → quality degradation." Ditto's default must pin precision (`Precision::Exact`, `allow_quantization: false`).

8. **Subscription auth is risk-fragmented; explicit policy metadata is required.** Anthropic's policy reversed twice in 90 days (Jan/Feb cutoff → April 4 ban → May 22 metered "Agent SDK credits" reversal). Tools must surface `PolicyStatus::{Allowed, GreyArea, EnforcedBlock}` on every subscription backend so users can opt in with eyes open, not be surprised by a quiet break.

### Three surprise findings worth noting

- **Geography ≫ model tier for latency.** Tokyo 2× slower than Ireland. Bigger gains from regional routing than from picking a faster tier. The router must understand region.
- **Semantic caching practitioner reality: 20–45% hit rate.** Not 95%. Agentic tool calls only see 5–15%. Exact-match cache wins 15–30% of traffic before any semantic layer. Ship exact-match first; semantic is opt-in.
- **Compiled gateways (Bifrost: 11μs at 5,000 RPS)** are the new yardstick. Python-based LiteLLM cannot match this. A Rust crate is well-positioned — Ditto's *transport layer* is structurally faster than the incumbents'.

---

## What the research says NOT to do

Each is concrete; the citation is in the sub-documents.

| Anti-pattern | Why | Source |
|---|---|---|
| **Ship a "Sign in with Claude" OAuth wizard** | Banned. Legal risk. | oauth.md §Anthropic |
| **YAML config DSL for routing** | Silent failures; LiteLLM's lesson | landscape.md §LiteLLM |
| **Default to a proxy mode that runs on every Python startup** | Supply-chain risk; March 2026 attack | landscape.md §LiteLLM, community.md §migration zeitgeist |
| **Flatten `cache_control` / reasoning blocks / extended thinking to a lowest common denominator** | Silently strips Anthropic features when routed to OpenAI-shape providers | landscape.md §provider divergence |
| **Trust OpenRouter's default provider routing** | Quantization defaults break CJK + degrade quality | landscape.md §OpenRouter |
| **Hardcode official-client OAuth IDs for any provider that's enforcing** | Anthropic's enforcement target | oauth.md §Anthropic |
| **Persist Copilot chat tokens to disk** | They expire fast; abuse-detection signal | oauth.md §Copilot |
| **Synchronous tool-schema generation on every call** | The 73% overhead pattern | landscape.md §JIT projection |
| **Stream as `Stream<String>` or events flattened to text** | Loses tool deltas, parallel-call ordering, reasoning blocks | landscape.md §streaming, community.md §LiteLLM bugs |
| **Route through *another* LLM call to decide which model** | 200–800ms latency + a failure mode | landscape.md §cost routing |
| **Re-implement every SSE parser** | Pointless work; genai already has 17 | landscape.md §Rust ecosystem |

---

## Three architectural deltas this drives

The detailed design lives in [`../architecture/models.md`](../architecture/models.md). Three changes worth previewing here:

### Delta 1: Subscription auth is a typed backend, not a config flag

```rust
pub trait SubscriptionBackend: Send + Sync {
    fn provider(&self) -> ProviderId;
    fn policy_status(&self) -> PolicyStatus;     // Allowed | GreyArea | EnforcedBlock
    fn rate_limit_class(&self) -> RateLimitClass;
    async fn ensure_token(&self) -> Result<AccessToken>;
    async fn login(&self, kind: LoginKind) -> Result<LoginOutcome>;
    fn revoke(&self) -> Result<()>;
}
```

Concrete impls in v0:
- `CopilotOAuth` — primary subscription wedge. Two-stage `gho` → chat-JWT exchange, in-memory only. `PolicyStatus::Allowed`.
- `CodexOAuth` — device-code flow, `~/.ditto/codex.json`, 30s refresh safety margin. `PolicyStatus::GreyArea`.
- `GeminiAiOneOAuth` — installed-app flow, free tier capable. `PolicyStatus::Allowed`.
- `ClaudeCodeOAuth` — **not enabled by default.** Constructable only via explicit "I accept the policy risk" config flag. `PolicyStatus::EnforcedBlock`. Documents the `CLAUDE_CODE_OAUTH_TOKEN` env-var escape that Anthropic supports.

### Delta 2: The `Provider` trait carries typed capabilities, never silently flattens

```rust
pub trait Provider {
    type Capabilities: CapabilitySet;
    fn capabilities(&self) -> Self::Capabilities;
    // ...
}

pub trait CapabilitySet {
    fn tool_calling(&self) -> ToolCallingShape;       // OpenAi | Anthropic | Gemini
    fn prompt_caching(&self) -> CachingMode;          // None | Implicit | ExplicitCacheControl
    fn reasoning(&self) -> ReasoningMode;             // None | Hidden | ExplicitBlocks
    fn multimodal(&self) -> MultimodalCaps;
    fn rate_limit_headers(&self) -> RateLimitHeaderShape;
}
```

A request that uses `cache_control` against an `Implicit`-caching provider produces a typed *warning at build time* (rejected at `build_request`) instead of being silently dropped. This is the LiteLLM #14293 / Vercel AI SDK `providerOptions` lesson encoded structurally.

### Delta 3: `ToolRegistry::project()` is first-class, with three modes

```rust
pub enum ProjectionMode {
    Inline,                                              // classic JSON schemas
    Search { index_tool: ToolId },                       // MCP Tool Search shape
    CodeExecution { entrypoint_module: String },         // Anthropic Nov-2025 pattern
}
```

The agent loop picks mode per turn based on token budget + provider capability + tool count. Schemas are content-hashed and deduped; no schema is serialized twice in the same turn. This is how the 73% overhead becomes 2%.

---

## OpenRouter as the BYOK default

A note on the user-added strategic point: **Ditto's default BYOK catalog is OpenRouter, not direct providers.** Rationale:

- One adapter, 200+ models (incl. Claude via Anthropic's relationship — no Claude-OAuth risk).
- Honest cost transparency that LiteLLM doesn't deliver (Ditto surfaces OpenRouter's 5.5% credit purchase fee per call).
- Pinned precision (`provider_routing: { precision: Exact, allow_quantization: false }`) as Ditto's default — avoiding the CJK-breaking quantization defaults documented in roo-cline #11325.

Direct providers (`AnthropicNative`, `OpenAiNative`, `BedrockNative`, `VertexNative`, `Cerebras`, `Groq`) ship as additional adapters when users want native features (Anthropic explicit cache_control, OpenAI batch API, Bedrock regional routing, Cerebras 2200 tps).

---

## Risks and unknowns

- **Anthropic policy reversed twice in 90 days.** May reverse again. The May 22 metered "Agent SDK credits" announcement may evolve into a clean 3rd-party path. Re-evaluate quarterly.
- **OpenAI's Codex tolerance could change.** OpenAI partnering with Cline suggests it's stable for now, but `openai/codex#8338` is open. Build the device-code flow now, document the risk.
- **Copilot abuse-detection thresholds are opaque.** Aider/continue/opencode users have not been mass-banned, but "scripted interactions" + "unusual usage" are flagged in GitHub community discussions #160013, #130825. The risk is per-user, not legal.
- **genai version drift.** We depend on its transports + Copilot adapter. If genai changes its trait surface, we cope. Worst case: fork the transports we use.
- **OpenRouter outages.** Three in eight months, no SLA. Direct-provider fallback path in the Router must work even when OpenRouter is down.

---

## What's next

1. Update [`../architecture/models.md`](../architecture/models.md) with the design that operationalizes these findings (in flight, same commit).
2. Scaffold `ditto-models` Rust crate against that architecture.
3. Build provider adapters in this order: OpenRouter → Anthropic native → OpenAI native → GitHub Copilot OAuth → Codex OAuth → Gemini Code Assist → Bedrock → Vertex.
4. Add a `models` Python eval-harness adapter so we can measure cost / latency / tool-schema overhead per provider on real benchmarks.

The three sub-documents are the citation-grounded source-of-truth; revisit them when specific design decisions are contested.
