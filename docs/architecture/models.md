# Models architecture

The architectural commitment for Ditto's `ditto-models` crate. Reasoning, benchmarks, and citations are in [`../research/models.md`](../research/models.md) and its sub-documents.

## Thesis

`ditto-models` is **Pareto-better than LiteLLM, Vercel AI SDK, Portkey, Helicone, and the entire Rust ecosystem** for the specific job of powering an agent loop that:

1. Prefers the user's existing subscriptions (GitHub Copilot first, Codex/Gemini second) over making them open new billing relationships.
2. Calls 20+ frontier APIs through one trait with typed-not-flattened capabilities (cache_control, reasoning, extended-thinking, structured outputs all survive routing).
3. Refuses to pay the 73% tool-schema overhead documented in hermes-agent #4379, via JIT projection with three modes (Inline / Search / CodeExecution).
4. Emits a single normalized typed event stream — never `Stream<String>` — preserving reasoning blocks, parallel tool-call ordering, and provider_executed merges.
5. Routes via typed Rust config (`FallbackChain`, `RetryClass`, `RateLimitWindow`), never YAML.
6. Surfaces cost honestly with explicit `cache_read_tokens`, `cache_write_tokens`, `reasoning_tokens`, `usd_breakdown` per call.

The Bet 3 wedge restated in light of Anthropic's 2026-04-04 ban: **subscription auth is the wedge, not Claude specifically.** GitHub Copilot OAuth (contractually clean, multi-model in one auth) is the headline; OpenRouter BYOK is the multi-provider catalog fallback. No "Sign in with Claude" wizard.

## What we own vs lean on

Per the [vendoring policy](../research/models.md#the-vendoring-policy), the split:

**Own (the moat):**
- `Provider` trait + capability matrix (no silent flattening)
- `SubscriptionBackend` trait with `PolicyStatus` metadata
- `ToolRegistry` with content-hashed schemas + three projection modes
- The typed `Event` ontology (text-delta / tool-call-start / tool-input-delta / tool-input-end / tool-result / reasoning-delta / cache-hit / finish)
- The typed `Router` (chains, fallbacks, retry classes, rate-limit tracker, cost policy)
- Cost accounting struct with explicit cache/reasoning/output line items
- The Copilot two-stage token exchange, Codex device-code flow, Gemini installed-app flow

**Lean on:**
- `genai` (jeremychone/rust-genai, v0.5, Apache-2/MIT) — SSE transports + already-shipped Copilot adapter. Treat as a transport layer like `reqwest`.
- `models.dev` — the shared catalog of model capabilities, context windows, prices, IDs. Consume as data with local override.
- `reqwest` + `tokio` + `serde` + `tracing` for plumbing.
- `secrecy` for token handling; `keyring` for OS keychain integration.
- `rmcp` for the MCP-tool import path (we already depend on it for the server side).

**Avoid:**
- LiteLLM as a runtime dependency or shape model — it's the cautionary tale, not the template.
- A YAML routing DSL — see `community.md` for the silent-failure pattern.
- Hand-rolled SSE parsers per provider — genai's are tested.
- Hardcoding the Anthropic Claude Code official-client OAuth ID — invites enforcement.

## Crate surface

```
ditto-models/
├── src/
│   ├── lib.rs
│   ├── auth/                   # Auth backends
│   │   ├── api_key.rs
│   │   ├── bearer.rs
│   │   ├── aws_sigv4.rs
│   │   ├── google_adc.rs
│   │   └── subscription/
│   │       ├── copilot.rs       # PRIMARY — gho → chat-JWT, in-memory
│   │       ├── codex.rs         # device-code flow, ~/.ditto/codex.json
│   │       ├── gemini.rs        # installed-app flow, free tier capable
│   │       └── claude_code.rs   # PolicyStatus::EnforcedBlock; user opt-in only
│   ├── provider/               # Provider impls
│   │   ├── trait.rs
│   │   ├── openrouter.rs        # DEFAULT BYOK CATALOG — 200+ models
│   │   ├── anthropic_native.rs  # native features (cache_control, batch)
│   │   ├── openai_native.rs     # batch, structured outputs
│   │   ├── bedrock.rs           # AWS sigv4
│   │   ├── vertex.rs            # GCP ADC, Anthropic-on-Vertex divergence
│   │   ├── gemini_api.rs
│   │   ├── cerebras.rs          # extra-fast OpenAI-compat
│   │   ├── groq.rs              # OpenAI-compat
│   │   ├── deepseek.rs          # OpenAI-compat with <think> tags
│   │   ├── ollama.rs            # local
│   │   └── openai_compat.rs     # generic OpenAI-shape adapter
│   ├── capabilities/           # Typed capability projections
│   ├── tools/                  # ToolRegistry + three projection modes
│   ├── stream/                 # Typed event ontology
│   ├── router/                 # Typed chains, fallbacks, rate-limit
│   ├── cost/                   # CallCost, CostBreakdown
│   ├── catalog/                # models.dev consumer + local overrides
│   └── client.rs               # top-level Client
└── tests/
    ├── capability_routing.rs
    ├── tool_projection.rs       # validates 73% → ~2% reduction
    ├── streaming_ontology.rs
    ├── subscription_auth.rs
    └── router_fallback.rs
```

## Provider trait

```rust
pub trait Provider: Send + Sync + 'static {
    type Capabilities: CapabilitySet;
    type Request: Send;

    fn id(&self) -> ProviderId;
    fn models(&self) -> &[ModelDescriptor];
    fn capabilities(&self) -> Self::Capabilities;

    fn build_request<Ext>(&self, call: &Call<Ext>) -> Result<Self::Request>
    where Ext: ProviderExtensions<Self::Capabilities>;

    async fn stream(
        &self,
        req: Self::Request,
        auth: &AuthHandle,
    ) -> Result<stream::EventStream>;
}

pub trait CapabilitySet {
    fn tool_calling(&self) -> ToolCallingShape;      // OpenAi | Anthropic | Gemini
    fn prompt_caching(&self) -> CachingMode;         // None | Implicit | ExplicitCacheControl
    fn reasoning(&self) -> ReasoningMode;            // None | Hidden | ExplicitBlocks
    fn multimodal(&self) -> MultimodalCaps;
    fn batching(&self) -> bool;
    fn native_web_search(&self) -> bool;
    fn rate_limit_headers(&self) -> RateLimitHeaderShape;
    fn structured_outputs(&self) -> StructuredOutputMode;
}
```

The key trick: `Call<Ext>` is parameterized on a typed extension struct, and `build_request` is bounded `where Ext: ProviderExtensions<Self::Capabilities>`. A request that includes `cache_control` against a provider whose `prompt_caching() == Implicit` won't satisfy the bound — it errors at *type-check time* if known at compile time, at *build-request time* if dynamic. This is the architectural answer to LiteLLM #14293 (Anthropic beta header → Vertex 400) and Vercel AI SDK's `providerOptions` opacity.

## SubscriptionBackend

```rust
pub enum PolicyStatus {
    Allowed,         // contractually clean (Copilot, Gemini)
    GreyArea,        // documented but TOS-fuzzy (Codex)
    EnforcedBlock,   // actively banned (Claude Code OAuth post-2026-04-04)
}

pub trait SubscriptionBackend: Send + Sync {
    fn provider(&self) -> ProviderId;
    fn policy_status(&self) -> PolicyStatus;
    fn rate_limit_class(&self) -> RateLimitClass;
    async fn ensure_token(&self) -> Result<AccessToken>;
    async fn login(&self, kind: LoginKind) -> Result<LoginOutcome>;
    async fn refresh(&self) -> Result<AccessToken>;
    fn revoke(&self) -> Result<()>;
}
```

v0 implementations:

| Backend | Status | Flow | Storage | Notes |
|---|---|---|---|---|
| `CopilotOAuth` | `Allowed` | Device flow → `gho_*` → chat-JWT exchange | gho in keychain; chat JWT in-memory only (expires <30 min) | Primary subscription wedge. Headers: `Editor-Version`, `Copilot-Integration-Id`. |
| `CodexOAuth` | `GreyArea` | Device-code (`/codex/device`) PKCE S256 | `~/.ditto/codex.json` via `keyring` | 30s safety-margin auto-refresh. |
| `GeminiOAuth` | `Allowed` | OAuth 2.0 installed-app | Local file with restrictive perms | Free tier: 60 rpm / 1000 rpd, no card. |
| `ClaudeCodeOAuth` | `EnforcedBlock` | Disabled by default. User opt-in via explicit `policy_acknowledged: true` config. | Recognizes `CLAUDE_CODE_OAUTH_TOKEN` env var as Anthropic-supported BYO-token path. | Will log a runtime warning every session. |

The CLI exposes `ditto auth login --provider copilot` (or codex/gemini); claude-code requires `--accept-policy-risk` and is omitted from `ditto auth wizard` output.

## ToolRegistry + JIT projection

```rust
pub struct ToolRegistry {
    by_id: HashMap<ToolId, Arc<Tool>>,
    schema_cache: HashMap<SchemaHash, Arc<Bytes>>,
}

pub struct TurnProjection<'a> {
    pub channel: Option<&'a str>,           // "telegram", "cli", "discord"
    pub allowed_kinds: &'a [ToolKind],
    pub budget_tokens: usize,
    pub mode: ProjectionMode,
    pub provider_caps: &'a dyn CapabilitySet,
}

pub enum ProjectionMode {
    /// Classic JSON schemas inline. Used when (a) tool count × schema size
    /// stays under budget, and (b) the provider doesn't support Search/Code.
    Inline,
    /// Single `tools.search()` shim tool; client fetches schemas JIT when
    /// the model invokes search. Mirrors Claude Code v2.1.7's pattern.
    Search { index_tool: ToolId },
    /// MCP-as-code-module: provider receives an import-style shim and the
    /// agent writes `gdrive.get_transcript(...)` instead of receiving the
    /// schema inline. Mirrors Anthropic's Nov 2025 pattern; 98.7% reduction.
    CodeExecution { entrypoint_module: String },
}

impl ToolRegistry {
    pub fn project(&self, turn: TurnProjection<'_>) -> Projected;
}
```

Properties enforced:
- Schemas are content-hashed (SHA-256 of canonical JSON, via `ditto-core::canonical`). Identical schemas across tools collide, dedupe automatically.
- No schema is serialized twice in the same turn — `Projected` holds `Cow<'a, [u8]>` from a per-request cache.
- Per-channel/role static filters run *before* projection — Telegram doesn't see `browser_*`, Discord doesn't see `email_*`.
- `Projection::CodeExecution` is selected automatically when (a) the provider's capability matrix supports MCP-as-code, (b) tool count > some threshold, (c) the user hasn't opted out.

The test `tool_projection.rs` validates that a 30-tool MCP setup produces <2% schema overhead under `CodeExecution`, vs the 73% baseline.

## Streaming event ontology

```rust
pub enum Event {
    TextDelta { text: String },
    ReasoningDelta { text: String, signature: Option<String> },
    ToolCallStart { id: String, name: String, index: u32 },
    ToolInputDelta { id: String, json_patch: String },
    ToolInputEnd { id: String },
    ToolResult { id: String, content: ToolResultContent, provider_executed: bool },
    CacheHit { read_tokens: u32 },
    CacheWrite { write_tokens: u32 },
    UsageInterim(Usage),
    Finish { reason: FinishReason, usage: Usage },
    ProviderError { code: ErrorClass, message: String, retryable: bool },
}
```

Properties enforced:
- **No `Stream<String>` anywhere.** That's the LiteLLM #20711 / #21331 failure mode.
- `ToolCallStart.index` is preserved even when providers don't expose it natively (we synthesize from order-of-arrival). This is the index=0 bug.
- `ReasoningDelta` is distinct from `TextDelta`. Anthropic extended-thinking, OpenAI o1 hidden, DeepSeek `<think>` tags all map here.
- `ToolResult.provider_executed: bool` so Mastra-style server-side-tool merging is trivial.
- `CacheHit` / `CacheWrite` / `UsageInterim` are first-class — never collapsed into `Finish.usage` (Anthropic's prompt-caching pricing has 10%/125% line items that must be visible per call).

## Router

```rust
pub struct Router {
    chains: Vec<Chain>,                       // typed, ordered
    rate_limits: RateLimitTracker,            // from observed headers
    cost_policy: CostPolicy,
    quality_policy: QualityPolicy,            // from models.dev benchmarks
    region_policy: RegionPolicy,              // geography ≫ tier for latency
}

pub struct Chain {
    pub name: String,
    pub primary: ModelRef,
    pub fallbacks: Vec<ModelRef>,
    pub retry: RetryPolicy,                   // typed per ErrorClass
    pub on_context_overflow: ContextOverflowPolicy,
    pub provider_routing: ProviderRoutingPolicy,
}

pub struct ProviderRoutingPolicy {
    pub precision: Precision,                  // default: Exact (refuse quantized)
    pub allow_quantization: bool,              // default: false
    pub region: Option<Region>,
    pub max_first_token_latency_ms: Option<u32>,
}

pub enum ErrorClass {
    RateLimit, ContextWindowExceeded, ContentPolicy,
    Auth, Timeout, InternalServer, NetworkTransient,
}
```

Defaults explicitly chosen against OpenRouter's "ship quantized providers by default" pattern. The `Precision::Exact` default refuses CJK-breaking FP4/Int4 backends documented in roo-cline #11325.

## Cost accounting

```rust
pub struct CallCost {
    pub input_tokens: u32,
    pub cache_read_tokens: u32,         // 10% of input cost on Anthropic
    pub cache_write_tokens: u32,        // 125% of input cost on Anthropic
    pub output_tokens: u32,
    pub reasoning_tokens: u32,          // billed separately on o1, extended-thinking, R1
    pub usd: f64,
    pub usd_breakdown: CostBreakdown,
}

pub struct CostBreakdown {
    pub input_usd: f64,
    pub cache_read_usd: f64,
    pub cache_write_usd: f64,
    pub output_usd: f64,
    pub reasoning_usd: f64,
    pub openrouter_fee_usd: f64,        // 5.5% credit purchase fee, surfaced honestly
}
```

Honest cost telemetry was the loudest OpenRouter pain in `community.md`. Surfacing each line item is the answer.

## Eval surface

`ditto-models` ships in-tree benchmarks:

- `tool_projection_overhead.rs` — measure tokens-per-call across Inline / Search / CodeExecution modes against a 30-tool fixture. Floor: <5% overhead on CodeExecution; <15% on Search.
- `streaming_fidelity.rs` — generate a fixture stream with parallel tool calls, mixed text+tool deltas, reasoning blocks. Verify every event arrives intact. Test against each provider's recorded stream.
- `capability_round_trip.rs` — assert that an `Anthropic` Call with `cache_control` errors clearly when routed to a `CachingMode::Implicit` provider (vs LiteLLM's silent strip).
- `subscription_oauth_smoke.rs` — gated tests for each backend; require env var.

## What we deliberately do not ship in v0

- HTTP proxy mode (`ditto-proxy` lives as a separate binary later, never as a default — supply-chain risk from LiteLLM's lesson)
- Sample-level shadow routing (run two providers, compare outputs) — interesting but feature-creep
- Quality-aware autorouting via online RL (RouteLLM-style) — track, don't build
- Microsoft 365 Copilot subscription path (no public OAuth for 3rd-party)
- Cursor / Perplexity / Mistral subscription auth (no public flows)
- Semantic caching layer — Exact-match cache only in v0 (the 20-45% hit rate community-research finding says semantic isn't worth the latency in v0)

## Open questions (re-evaluate at month 6)

- Anthropic's policy may reverse again (May 22 2026 "Agent SDK credits" hints at a clean 3rd-party path). If so, `ClaudeCodeOAuth` could move from `EnforcedBlock` to `Allowed`. Re-evaluate quarterly.
- OpenAI's Codex tolerance — `openai/codex#8338` open. If a clear permission is granted, lift the `GreyArea` flag.
- `genai` API stability — the Rust ecosystem is young; if breaking changes break us, vendor the transports we depend on.
- OpenRouter outage SLA — three in 8 months, no SLA. Direct-provider fallback must work even when OpenRouter is down. Test under fault injection.
- Compiled-gateway performance bar — Bifrost: 11μs at 5,000 RPS. Ditto's transport layer should match or beat. Measure before claiming.

## Related

- [`../research/models.md`](../research/models.md) — synthesis entry point
- [`../research/models/landscape.md`](../research/models/landscape.md) — LiteLLM, Vercel AI SDK, Rust ecosystem, provider divergence
- [`../research/models/oauth.md`](../research/models/oauth.md) — every subscription-OAuth flow, TOS analysis
- [`../research/models/community.md`](../research/models/community.md) — practitioner pain catalog, OSS issue clusters
- [`./memory.md`](./memory.md) — Ditto's memory architecture (`ditto-memory` is the moat above this layer)
- [`./multi-tenant.md`](./multi-tenant.md) — Org / Tenant / Workspace / Matter (the auth-tenancy hierarchy `SubscriptionBackend` plugs into)
