# ditto-models: research for a Rust LLM router

Research compiled 2026-05-14 for the Ditto agent harness. Scope: design a `ditto-models` Rust crate that is Pareto-better than LiteLLM, Vercel AI SDK, Portkey, Helicone, OpenRouter SDK, async-openai, rig, and genai for the specific job of powering an agent loop that prefers the user's existing Claude/Codex/Copilot subscriptions, calls 20+ frontier APIs, and refuses to pay the 73% tool-schema overhead documented in hermes-agent #4379.

---

## 1. Executive summary — the five things `ditto-models` must get right

1. **Treat subscription OAuth as a first-class, deprecation-risk-aware backend.** Anthropic enforced the third-party ban on 2026-04-04 and sent a legal request to OpenCode (HN 46549823, theregister 2026-02-20). Codex OAuth is still tolerated. Copilot OAuth is grey. Ditto should ship `SubscriptionAuth::{ClaudeCode, Codex, Copilot, GeminiOneAI}` as named backends with explicit "policy class" metadata — Anthropic-style "official-client-only" is a different risk than Codex-style "documented OAuth". Refuse to lie to users that a banned backend is supported; expose `policy_status: Allowed | GreyArea | EnforcedBlock` from the registry.
2. **The provider trait must not be a lowest-common-denominator interface.** LiteLLM and Vercel AI SDK both flatten Anthropic `cache_control`, extended-thinking, reasoning effort, thought signatures, and provider-executed tools into shapes the next provider can't refuse. The result is bugs like LiteLLM #14293 (Vertex rejects Anthropic beta header), #21331 (parallel tool calls all emit index=0), #20711 (90% of argument-delta events dropped), #17246 (mixed text+tool stream loses the tool deltas). Ditto's `Provider` trait must expose typed `ProviderCapability` projections and let typed extensions ride through, not be silently flattened.
3. **JIT tool-schema projection is non-negotiable.** Anthropic's own engineering blog (Nov 2025, "Code execution with MCP") shows a 150k-token workflow compressed to 2k tokens — a 98.7% reduction — by treating MCP servers as a code-loadable filesystem. Hermes-agent #4379 measures 13,935 tokens of fixed overhead per call (73% of every API call). Claude Code 2.1.7 ships MCP Tool Search that activates when tool defs exceed 10% of context. Ditto must expose `ToolRegistry` as a *queryable index*, not an array preloaded onto every call; per-channel filtering, content-hash dedup of repeated schemas, and a "code-execution mode" surface where the model receives a tiny `tools.search()` shim instead of N inline schemas.
4. **Streaming must be a single normalized event stream with typed deltas, not a `String` async iterator.** Practitioners burn weeks on this. Anthropic interleaves text and tool deltas; OpenAI declares tool-calls upfront with index-keyed deltas; Gemini sometimes returns `finish_reason=stop` when it should be `tool_calls` (LiteLLM #21041). Reasoning models stream `<think>` content separately. Vercel AI SDK 6 finally got `tool-call-delta`/`tool_input_delta`/`tool_input_end` events; that ontology is the right floor. Above it, ditto must preserve reasoning blocks, cache-read/write counts, and provider-executed tool merges.
5. **Cost, retries, fallback, and rate-limit handling belong in a typed router, not a YAML DSL.** LiteLLM's YAML is the *de facto* standard but produces silent failures at scale (issue #6345 "performance degrades over time, fixed by restart"; multiple "fake stream" fallbacks at #21090). Ditto's router should be a typed Rust struct with `Policy<Model>`, `FallbackChain`, `RetryClass`, `RateLimitWindow`, observable via tracing spans. Cost accounting must include explicit cache-read vs cache-write line items (Anthropic charges 10% read / 125% write) and reasoning-token cost separately.

---

## 2. Library landscape

### LiteLLM (Python) — incumbent, but bleeding
- ~22k stars, the reference router. Supports ~100 providers, OpenAI-compatible proxy, YAML config, fallback chains, content-policy fallbacks, usage-based routing. `routing_strategy: simple-shuffle | least-busy | usage-based-routing | latency-based-routing`.
- **Production-grade pain**: supply chain compromise March 24 2026 (versions 1.82.7/1.82.8 published with credential-stealing payload via `.pth` injection — see Datadog Security Labs, Snyk, Cycode, Trend Micro). Stolen via Trivy CI/CD compromise. Lesson for ditto: minimize the surface that runs *every Python startup*, and never proxy through a package that escalates implicit privileges on import.
- **Streaming/tool-call bug cluster**: #20711, #17246, #21331, #19744, #21090, #21041 — all 2025-2026 bugs about losing tool-call deltas or misreporting finish reasons. LiteLLM tries to be both shape-translator and proxy; the shape translation lossy.
- **Caching abstraction leaks**: #14293 (Anthropic beta header always added when routed to Vertex, which rejects it). #15943 (Databricks Anthropic models broken). #9805 (no default Bedrock cache enable). `cache_control` "works for Gemini/Vertex but is ignored elsewhere" — silent.
- **Perf regression**: #6345 "Performance degradation over time, fixed by service restart" — open since 2024.
- License: MIT (proxy is now BSL for some features).

### Vercel AI SDK (TS) — best-in-class typed surface, weakest extensibility
- ~12k stars, currently AI SDK 6 with `Agent` abstraction, `generateText`, `streamText`, `generateObject`, `streamObject`, typed `tool()` defs with Zod schemas. `tool-call-delta`/`tool_input_delta`/`tool_input_end` streaming ontology is the right model for streaming tool args.
- **Strengths**: typed everywhere; `experimental_providerMetadata` carries Anthropic `cacheControl` (after a long fight — see OpenRouterTeam/ai-sdk-provider#35). Provider executed-tool merging in AI SDK 5+ is excellent.
- **Weaknesses**: provider-specific features punted to `providerOptions` bags (cache_control is exclusive to Anthropic SDK or providerOptions, and is *easy* to forget). Resumable streams + abort signals incompatible (own docs). 1-2ms overhead per call vs raw SDK.
- License: Apache 2.0.

### Portkey vs Helicone vs OpenRouter (managed)
- **Portkey**: 1,600 LLMs, governance-first. ~$49/mo. Adds 20-40ms latency. Good failover/caching/multi-tenant guardrails.
- **Helicone**: Rust proxy, P50 8ms / P95 <5ms, 3000 RPS on 64MB. Observability-first; routing secondary. Open-source, free to self-host.
- **OpenRouter (service)**: documented quality-drift problems. Defaults to mixed quantization providers (Venice, NextBit, Mancer, DeepInfra) — community complaints in RooCodeInc/Roo-Code#11325 (CJK encoding broken on FP4/Int4), QwenLM/qwen-code#348 ("avoid quantized models on OpenRouter"). Three outages in eight months, no SLA. 5.5% credit purchase fee. *Their own Exacto launch* concedes "providers tweak inference stacks → quality degradation." Ditto must let users *pin precision*.

### OpenAI Agents SDK + LiteLLM extension
- Python + TS. `LitellmModel` extension lets you point at any LiteLLM-supported provider. Works, but inherits all LiteLLM streaming bugs. `extract_all_content` drops parameters OpenAI doesn't know, so Anthropic-only fields silently disappear (openai-agents-python #1257).

### Anthropic SDK (native, Bedrock, Vertex)
- Three flavors. Vertex/Bedrock lag native by *weeks* (extended thinking, MCP connectors, structured outputs). Vertex/Bedrock have different region semantics; Vertex regional endpoints are +10% over global. ditto must encode this lag explicitly.

### langchain + llamaindex
- Both have LLM abstractions. Both flatten provider features. Practitioners on Reddit consistently report churn on cache-control and tool-call shape changes. Not a useful base.

### Cline, opencode, aider, continue.dev, roo-cline
- All converged on **models.dev** as the shared catalog of model capabilities, context windows, prices, and IDs. `providerId/modelId` is the de-facto reference syntax. *Ditto should consume models.dev as a data source* (with cache + override), not maintain its own registry.
- Continue uses `config.yaml` with provider blocks. Cline's TS code now supports 30+ providers with per-provider edit cases. Aider's `architect/editor` decoupling is *orthogonal* to provider abstraction (different problem) but its `edit_format` config is the canonical example of "models have personalities; pick the format that works."
- Roo-Cline forks accumulate provider-specific fixes faster than upstream — strong signal that *any* fixed provider trait without an escape hatch will rot.

### Mastra
- TS agent framework. Tool streaming with `context.writer` is the right pattern for "tool emits progress into the stream." Provider-executed tools merged back into the originating tool call. AI-SDK-based.

---

## 3. Rust ecosystem reality

| Crate | Latest | Providers | Streaming | Tools | Caching | Reasoning | OAuth | Verdict |
|---|---|---|---|---|---|---|---|---|
| `async-openai` | maintained | OpenAI only (OpenAPI spec-faithful) | yes | yes | no | partial | none | Strong base for OpenAI shape; many forks (`async-openai-wasm`, `async-openai-compat`, `async-openai-alt`) — divergence is itself a signal. |
| `rig-core` v0.37 (May 13 2026) | active | 20+ | streaming agents | yes | not exposed | not exposed | none | MIT, used by Coral, St Jude, Neon, Nethermind. Owner explicitly warns "future updates *will* contain breaking changes." Modular and ergonomic but capability surface is shallow. |
| `genai` v0.5 (jeremychone) | active | OpenAI, Anthropic, Gemini, xAI, Ollama, Groq, DeepSeek, Cohere, Together, Fireworks, Nebius, Mimo, Zai, BigModel, Aliyun, Vertex, **GitHub Copilot** | yes (unified EventSourceStream) | yes | partial | "Gemini Thinking + Anthropic Reasoning Effort" support | Custom auth/endpoint config | 766 stars; Apache-2/MIT dual. Most provider breadth of any Rust crate. Tool-calling support arrived only recently. Already shipped a Copilot adapter — relevant prior art. |
| `graniet/llm` | maintained | OpenAI, Claude, Gemini, Ollama, DeepSeek, xAI, Phind, Groq, Cohere, Mistral, HF, ElevenLabs | yes | yes | unclear | unclear | none | Smaller, includes TTS/STT/vision. Builder-style. |
| `litellm-rust` | nascent | port-in-progress | partial | partial | n/a | n/a | n/a | Aspirational; not production. |
| `swiftide` | active | OpenAI, Anthropic, Ollama | yes | yes | n/a | n/a | none | Agent + ingestion framework; routing is incidental. Heavy breaking changes acknowledged. |
| `llm-chain` v0.12 | low activity | multiple | partial | partial | no | no | none | LangChain-shaped; not the right architecture. |
| `ferrochain` | nil | — | — | — | — | — | — | No findings; treat as nonexistent. |
| `rustformers/llm` | **unmaintained** | local only | — | — | — | — | — | Dead. |

**Verdict**: there is no Rust crate close to LiteLLM-quality. The closest in *capability surface* is `genai` (breadth, Copilot adapter, reasoning support) and the closest in *ergonomics* is `rig`. Both leak provider-specific knobs through opaque options; neither has subscription OAuth as a first-class concept; neither has a typed router config; neither addresses JIT tool-schema projection. Gap to "production multi-provider router": ~6-9 months of focused work, *or* a fork of genai with the trait redesigned around typed capabilities and OAuth backends.

The pragmatic choice: **`ditto-models` should depend on `genai` for provider transports** (don't reinvent SSE parsers for 20+ APIs) but own its own `Provider` trait, capability model, OAuth flows, router, tool registry, and event stream. Treat genai's adapters as a transport layer like reqwest.

---

## 4. Subscription OAuth deep dive

### 4.1 Claude Code OAuth (high risk)
- OAuth 2.1 authorization-code flow with PKCE (S256, no client secret).
- Endpoint discovery via `.well-known/oauth-authorization-server`; Anthropic auth server is at `console.anthropic.com` / `auth.anthropic.com`.
- Hardcoded `client_id` for the official Claude Code CLI (gist by shubcodes documents the manual flow, binary analysis, 15s timeout bypass, macOS keychain storage at `~/.claude/`).
- Access token TTL ~8 hours; refresh token long-lived; 401 → use refresh.
- **TOS reality**: theregister 2026-02-20, venturebeat, mindstudio, kersai.com — Anthropic clarified Feb 20 2026 that OAuth from Free/Pro/Max/Team/Enterprise plans is for "ordinary individual use of Claude Code and other native Anthropic apps." Server-side blocks deployed Feb–March 2026. Full enforcement April 4 2026 3PM ET. OpenCode received a legal request and removed support. HN 46549823 + 47069299 captured the community discussion.
- **Implementation hint for ditto**: ship the backend but mark it `policy_status: EnforcedBlock`. Surface a documented user opt-in path that says "Anthropic enforcement may revoke this without notice." Do not enable by default. The honest design is to *not* impersonate the official Claude Code client_id — that's the line Anthropic is enforcing on.

### 4.2 Codex / ChatGPT OAuth (tolerated as of 2026-05)
- Two flows: browser callback + device-code (`{issuer}/codex/device`).
- PKCE S256, but authlib does *not* auto-add `code_challenge`; must be generated with `secrets`+`hashlib` (Logto blog, codex-rs/server in `persist_tokens_async`).
- Token storage `~/.codex/auth.json`. Tokens auto-refresh during use; `SAFETY_MARGIN` = 30s. `auto_refresh` pre-checks validity and retries once on 401.
- Client ID is the Codex CLI's *reused* by OpenCode (per OpenAI dev community thread). No third-party allocation mechanism.
- TOS: "Codex is included with ChatGPT Plus, Pro, Business, Enterprise/Edu; sign in with ChatGPT routes to subscription limits and Codex cloud." OpenAI has *not* matched Anthropic's enforcement (as of May 2026).
- **Implementation hint**: device-code flow is the cleanest for headless agent harnesses; ship that path first, then browser flow.

### 4.3 GitHub Copilot OAuth (grey)
- VSCode uses `gho_*` OAuth-app tokens, then exchanges for a Copilot Chat completion token (short-lived JWT).
- Reverse-engineered by ericc-ch/copilot-api, caozhiyuan/copilot-api, templarsco/opencode-copilot-bridge. Now officially supported by OpenCode (github.blog/changelog 2026-01-16: "GitHub Copilot now supports OpenCode").
- Aider, opencode, continue all consume Copilot via the same path. genai already ships an adapter.
- TOS: "abuse-detection systems flag activity that includes use of Copilot via scripted interactions, deliberately unusual or strenuous usage, or multiple accounts to circumvent usage limits" (orgs/community discussion #160013, #130825). Risk: temporary suspension of Copilot, not legal action seen yet. Org admins can block third-party MCP/CLI flows.
- **Implementation hint**: implement the two-stage token exchange, cache the chat token in memory only (don't persist; it expires fast), respect `X-RateLimit` headers conservatively. Document the risk in the README.

### 4.4 Google AI Studio / Gemini OAuth
- Standard OAuth 2.0 with `client_secret.json` and `installed app flow`; scope `https://www.googleapis.com/auth/generative-language.retriever`.
- Subscription routing conflict documented in google-gemini/gemini-cli#19970: consumer Google One AI Pro entitlement overrides Enterprise Gemini Code Assist Standard, "stripping proprietary IP guarantees." ditto must let users *pick* which entitlement.
- Gemini CLI has separate auth modes; OpenCode has a plugin for the consumer subscription (syntackle.com blog).

### 4.5 Microsoft / M365 Copilot
- No public OAuth flow for consumption that lets third-party tools draw from a user's M365 Copilot subscription. Enterprise plans require admin-managed app registrations through Entra. Not viable for an open-source agent harness in 2026.

### 4.6 OAuth surface for ditto
```rust
pub enum SubscriptionAuth {
    AnthropicClaudeCode { client_id: String, redirect_uri: String, scopes: Vec<String> },
    OpenAiCodex { device_flow: bool },
    GithubCopilot { app_token_path: Option<PathBuf> },
    GoogleAiOne { client_secret_path: PathBuf, scope: String },
}
pub enum PolicyStatus { Allowed, GreyArea, EnforcedBlock }
pub trait SubscriptionBackend {
    fn policy_status(&self) -> PolicyStatus;
    async fn ensure_token(&self) -> Result<AccessToken>;
    async fn refresh(&self) -> Result<AccessToken>;
    fn rate_limit_class(&self) -> RateLimitClass;
}
```

---

## 5. Provider divergence map

Concrete per-feature incompatibilities ditto must encode:

| Feature | OpenAI | Anthropic | Gemini | Bedrock | Vertex | DeepSeek | Moonshot | xAI |
|---|---|---|---|---|---|---|---|---|
| Tool call shape | `tool_calls[]` array, JSON args streamed by index | `tool_use` blocks interleaved with text | `function_call` parts | as Anthropic native + beta-header divergence | as Anthropic native + 10% surcharge + region split | OpenAI-shape | OpenAI-shape (Anthropic-compat endpoint exists) | OpenAI-shape |
| Streaming delta | text + `tool_calls[index].function.arguments` | `content_block_start/delta/stop` for text *and* tool | `candidates[].content.parts[]` chunks | varies by inference profile | as Anthropic | OpenAI-shape | OpenAI-shape | OpenAI-shape |
| Cache control | implicit, auto (≥1024 tok, 5–10min TTL) | explicit `cache_control: {type:"ephemeral"}`, up to 4 breakpoints, tools→system→messages order | implicit context caching | Anthropic explicit, beta header required | Anthropic explicit *but rejects same beta header* — LiteLLM #14293 | implicit, ~$0.014/M cache read | implicit (Kimi via Groq), GLM-5 caches prefixes at 1/5 price | implicit |
| Reasoning tokens | `o1`/`o3`: `reasoning_effort`, hidden tokens, billed | `extended_thinking` blocks, optionally returned, can be cached only when in prior assistant turn | `thinking` mode | propagated | propagated | `<think>` tags in stream | varies | `reasoning_content` |
| Stop sequences | `stop[]` | `stop_sequences[]` | `stopSequences[]` | as Anthropic | as Anthropic | as OpenAI | as OpenAI | as OpenAI |
| Temperature | 0–2 | 0–1 | 0–2 | model-dep | model-dep | 0–2 | 0–2 | 0–2 |
| Multimodal | image, audio (Realtime), video (Sora API) | image, PDF (file API) | image, audio, video natively | image | image | image | image (Kimi-VL) | image |
| File API | yes | yes (Files beta) | yes | indirect via S3 | indirect via GCS | no | partial | no |
| Native web search | `web_search_preview` tool | `web_search` server tool | grounding with Google Search | no | no | no | no | `live_search` |
| Batch API | yes | yes | yes | yes | yes | partial | no | no |

**Where abstractions leak (specific evidence)**:
- LiteLLM strips Anthropic `cache_control` when routed to non-Anthropic — silent (own docs).
- LiteLLM #14293: Anthropic beta header always set, rejected by Vertex.
- LiteLLM #21331: parallel tool-call deltas all emit index=0 in the bridge.
- LiteLLM #21090: custom models fall back to fake streaming; function_call events dropped silently.
- Vercel AI SDK provider modules need explicit `providerOptions.anthropic.cacheControl` per message — easy to miss; community issue 35 in OpenRouter ai-sdk-provider documents months of confusion.
- OpenAI Agents SDK `extract_all_content` drops non-OpenAI fields.

**Design rule**: ditto's provider trait carries a `ProviderRequest<P: Provider>` parameterized over the provider type. Cross-provider features go in a *typed extension struct* with `#[provider(anthropic = "cache_control", openai = "ignore")]` attributes; if an extension is `ignore`, the user gets a typed warning, not silent loss.

---

## 6. JIT / lazy tool-schema projection

The hermes-agent #4379 thread is the strongest existing evidence: 13,935 tokens of fixed overhead per call, 73% of every API call, with `browser_*` tools loaded on platforms where they can't be invoked. The skills index alone is 2,200 tokens, *replicated* even when accessible via `skill_view`/`skills_list`.

State of the art techniques (best-first):

1. **Code-execution mode (Anthropic Nov 2025 blog)** — present MCP servers as importable modules in an execution environment (Pyodide, ditto's own JS/Python sandbox). Agent writes `import gdrive; gdrive.get_transcript(...)` instead of receiving the schema inline. Cited 98.7% token reduction (150k → 2k). This is the strongest pattern; ditto should treat it as the *default* for high-schema toolchains.
2. **MCP Tool Search (Claude Code v2.1.7)** — index is a single small tool; activates when MCP schemas would consume >10% of context. Tool descriptions live on the server; client fetches schemas just-in-time when the model invokes the search.
3. **Per-channel/role static filtering** — hermes-agent's `platform_toolsets` map. Telegram doesn't get `browser_*`. Trivial to implement, big wins; cline/opencode do per-mode filtering already.
4. **Schema deduplication via shared refs** — JSON Schema `$ref` to a `#/definitions/` block deduped across tools. Saves 30-50% when tools share types. MCP SEP-1576 ("Mitigating Token Bloat in MCP: Reducing Schema Redundancy") tracks this protocol-level.
5. **Token-budget-aware tool selection** — model picks a toolkit first (a single "router" call with descriptions only), then second call loads the chosen toolkit. FrugalGPT cascade applied to *tools* not *models*. RouteLLM-style training on tool-use data is open.
6. **Pi-MCP-Adapter / mcp-cli** — single proxy tool, ~200 tokens; client dynamically discovers downstream tools. Lower ceiling than code-execution mode but simpler.

**For ditto**: the `ToolRegistry` API should:
- store every tool's schema *content-hashed* and *deduped*;
- support `TurnProjection { channel, role, budget_tokens, allow_code_mode }`;
- expose `Projection::Code { entrypoint_module }` as a first-class output so the agent loop can decide;
- never serialize a tool schema to bytes twice in the same turn — return a `Cow<&'a [u8]>` from a per-request cache.

---

## 7. Cost / fallback / retry production patterns

- **Cascade**: cheap-model → quality check → escalate. FrugalGPT shows 98% cost reduction matching GPT-4. RouteLLM (lmsys) trained routers achieve 85% cost reduction on MT-Bench, 45% on MMLU.
- **Quality-aware**: pick by benchmark per task class (code → Claude/Sonnet; chat → cheap; vision → Gemini). models.dev has the benchmark metadata, ditto should consume it.
- **Latency-aware**: cache hits cheap, fresh expensive. Anthropic cache write costs 25% more than input, read 10%. Plan prompts so the cacheable prefix dominates.
- **Failover**: when provider rate-limited, route to a same-family fallback. LiteLLM's pattern: `fallbacks: [{"zephyr-beta": ["gpt-4o"]}]`, with `RateLimitErrorRetries`, `TimeoutErrorRetries`, `ContentPolicyViolationErrorRetries`, `InternalServerErrorRetries`, `AuthenticationErrorRetries`, `context_window_fallbacks` separately. Routing strategies: `simple-shuffle`, `least-busy`, `usage-based-routing`, `latency-based-routing`.
- **Rate-limit headers**: Cerebras returns `x-ratelimit-{limit,remaining,reset}-{requests,tokens}-{minute,day}`; Groq the same shape; NVIDIA NIM less detailed; OpenAI standard. Anthropic returns `anthropic-ratelimit-*`. *ditto must proactively track and respect these.*
- **Quota observability**: opencode-quota project shows the value of a per-provider quota dashboard separate from context-window pollution.

**Anti-pattern not to adopt**: routing through *another* LLM call to decide which model — adds 200-800ms and a failure mode. Use cheap features (heuristics, embeddings, perplexity) when possible.

---

## 8. Community pain catalog — quoted, sourced

1. **"LiteLLM Performance Degradation Over Time Fixed by Service Restart"** — BerriAI/litellm#6345. "After 2–3 hours it gets slower." Open. Implication: ditto must have explicit, observable connection/cache lifecycle.
2. **"Proxy streaming fails to emit tool_calls when openai/responses/* models return multi-output"** — BerriAI/litellm#17246. Mixed text+tool stream drops the tool deltas.
3. **"Responses API Streaming Drops Tool Call Argument Deltas"** — BerriAI/litellm#20711, severity 8/10. "~90% of argument delta events are lost."
4. **"Parallel tool calls all emit index=0"** — BerriAI/litellm#21331. Bridge can't distinguish which delta belongs to which call.
5. **"Gemini 3 Flash returns finish_reason=stop instead of tool_calls"** — BerriAI/litellm#21041. Downstream agents misjudge end of stream.
6. **"Anthropic Beta Headers Not Forwarded to Vertex AI Claude Models"** — BerriAI/litellm#15299.
7. **"Cache-enabled Anthropic requests routed to Vertex always fail due to invalid cache header"** — BerriAI/litellm#14293.
8. **"Anthropic caching not supported on LiteLLM"** — openai/openai-agents-python#1257.
9. **"73% of every API call is fixed overhead (~13.9K tokens)"** — NousResearch/hermes-agent#4379. The headline data point for ditto's JIT-projection thesis.
10. **"Codex does not auto-refresh routed MCP OAuth tokens"** — openai/codex#17265.
11. **"Anthropic officially bans using subscription auth for third party use"** — HN 47069299, theregister 2026-02-20.
12. **"Anthropic blocks third-party use of Claude Code subscriptions"** — HN 46549823. OpenCode legal request, removed support.
13. **"GitHub Copilot abuse-detection systems flag Copilot via scripted interactions"** — orgs/community#160013, #130825.
14. **"OpenRouter routes to FP4/Int4 quantization, breaks CJK"** — RooCodeInc/Roo-Code#11325, QwenLM/qwen-code#348.
15. **"OpenRouter returned 401 'User not found' during what was actually an infrastructure failure"** — production assessment, costing engineers hours.
16. **"OAuth flow forces consumer Google One AI Pro entitlement over Enterprise Gemini Code Assist Standard"** — google-gemini/gemini-cli#19970. Strips proprietary IP guarantees.
17. **"Vercel AI SDK doesn't expose cache_control on content blocks"** — pkgpulse guide 2026; partially mitigated via `providerOptions` but undiscoverable.
18. **"Resumable streams + abort signals are incompatible"** — Vercel own troubleshooting docs.
19. **"OpenRouter quality drift; introduced 'Exacto' to address provider variance"** — OpenRouter announcements page.
20. **"LiteLLM CI/CD compromised via Trivy, payload via .pth on every Python startup"** — Datadog Security Labs, Snyk, Cycode, Trend Micro reports, March 2026. Reinforces ditto-as-library (not proxy) preference.

---

## 9. What Ditto should adopt, build, and avoid

### Adopt
- **models.dev as catalog** — pull capabilities/prices/IDs from it, override locally.
- **OpenCode/Cline `providerId/modelId` reference syntax** — already user-known.
- **AI SDK 5/6 streaming ontology**: `text-delta`, `tool-call-start`, `tool-input-delta`, `tool-input-end`, `tool-result`, `reasoning-delta`, `finish` with usage payload.
- **LiteLLM's retry classes** as a typed enum: `RateLimitError`, `ContextWindowExceeded`, `ContentPolicy`, `Auth`, `Internal`, `Timeout`.
- **Anthropic code-execution-with-MCP pattern** for tool projection.
- **Codex device-code flow** for headless OAuth.
- **rate-limit header observation** (Cerebras/Groq/OpenAI/Anthropic shapes).
- **AI SDK provider-executed-tool merge semantics** — supports gateway tools cleanly.

### Build
- A typed `Provider` trait with capability projection (`CapabilitySet` per provider, never silently dropped).
- A `SubscriptionBackend` trait with `PolicyStatus` metadata.
- A `ToolRegistry` with content-hashed schemas and turn-level projections (including code-execution mode).
- A `Router` as typed Rust config (not YAML), with cost/latency/quality policies, `FallbackChain`, observable via `tracing::span`.
- Cost accounting struct with explicit `cache_read_tokens`, `cache_write_tokens`, `reasoning_tokens`, `usd_breakdown` per call.
- A *single* normalized event stream of typed deltas, never `Stream<String>`.

### Avoid
- A YAML DSL for routing. It's a *de facto* standard but fundamentally untyped and the cause of many silent failures.
- A proxy mode shipped as default (LiteLLM's lesson — supply-chain risk, lifecycle bugs). Ship a library; a proxy can be a thin separate binary.
- Flattening `cache_control` / extended-thinking / reasoning to a lowest common denominator.
- Re-implementing every SSE parser; depend on `genai` or `reqwest-eventsource` for transport.
- Hardcoded official-client OAuth IDs for Anthropic Claude Code — invite enforcement; respect the line.
- "LLM-as-judge for every call" router patterns. Cost and latency overhead is not worth it.
- Storing Copilot chat tokens to disk. They expire fast; keep them in memory.
- Letting OpenRouter default to quantized providers — `provider_routing: { precision: Exact, allow_quantization: false }` must be the ditto default.

---

## 10. Concrete API sketch for `ditto-models`

```rust
//! ditto-models — typed multi-provider LLM client for Ditto.
//!
//! Goals:
//!   * Subscription OAuth as first-class backends with policy_status.
//!   * Provider trait carries typed capability projections; no silent loss.
//!   * JIT tool-schema projection (per-channel, code-mode, content-hash dedup).
//!   * Streaming = one normalized event ontology.
//!   * Router as typed config, observable via tracing.

pub mod provider;
pub mod auth;
pub mod tools;
pub mod stream;
pub mod router;
pub mod cost;

// ---- providers ------------------------------------------------------------
pub trait Provider: Send + Sync + 'static {
    type Request: Send;
    type Capabilities: CapabilitySet;

    fn id(&self) -> ProviderId;                          // "anthropic", "openai", ...
    fn models(&self) -> &[ModelDescriptor];              // sourced from models.dev + overrides
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
    fn tool_calling(&self) -> ToolCallingShape;          // OpenAi | Anthropic | Gemini
    fn prompt_caching(&self) -> CachingMode;             // None | Implicit | ExplicitCacheControl
    fn reasoning(&self) -> ReasoningMode;                // None | Hidden | ExplicitBlocks
    fn multimodal(&self) -> MultimodalCaps;
    fn batching(&self) -> bool;
    fn native_web_search(&self) -> bool;
    fn rate_limit_headers(&self) -> RateLimitHeaderShape;
}

// ---- auth -----------------------------------------------------------------
pub enum AuthBackend {
    ApiKey(SecretString),
    BearerToken(SecretString),
    AwsSigV4(AwsCreds),
    GoogleAdc(GoogleCreds),
    Subscription(Box<dyn SubscriptionBackend>),
}

pub trait SubscriptionBackend: Send + Sync {
    fn provider(&self) -> ProviderId;
    fn policy_status(&self) -> PolicyStatus;             // Allowed | GreyArea | EnforcedBlock
    fn rate_limit_class(&self) -> RateLimitClass;
    async fn ensure_token(&self) -> Result<AccessToken>;
    async fn login(&self, kind: LoginKind) -> Result<LoginOutcome>;  // device-code | browser
    fn revoke(&self) -> Result<()>;
}

// concrete impls
pub struct CodexOAuth { /* device-code, ~/.ditto/codex.json, 30s safety margin */ }
pub struct CopilotOAuth { /* gho exchange → in-memory chat token, no disk */ }
pub struct ClaudeCodeOAuth { /* opt-in only, PolicyStatus::EnforcedBlock */ }
pub struct GoogleAiOneOAuth { /* installed-app flow */ }

// ---- tools ----------------------------------------------------------------
pub struct ToolRegistry {
    by_id: HashMap<ToolId, Arc<Tool>>,
    schema_cache: HashMap<SchemaHash, Arc<Bytes>>,       // content-hashed dedup
}

pub struct TurnProjection<'a> {
    pub channel: Option<&'a str>,                        // "telegram", "cli", "discord"
    pub allowed_kinds: &'a [ToolKind],                   // role/permission filter
    pub budget_tokens: usize,
    pub mode: ProjectionMode,
}
pub enum ProjectionMode {
    Inline,                                              // classic JSON schemas
    Search { index_tool: ToolId },                       // MCP Tool Search shape
    CodeExecution { entrypoint_module: String },         // Anthropic Nov-2025 pattern
}

impl ToolRegistry {
    pub fn project(&self, turn: TurnProjection<'_>) -> Projected;
}

// ---- streaming ------------------------------------------------------------
pub enum Event {
    TextDelta { text: String },
    ReasoningDelta { text: String, signature: Option<String> },
    ToolCallStart { id: String, name: String, index: u32 },
    ToolInputDelta { id: String, json_patch: String },
    ToolInputEnd { id: String },
    ToolResult { id: String, content: ToolResultContent, provider_executed: bool },
    CacheHit { read_tokens: u32 },
    Finish { reason: FinishReason, usage: Usage },
    ProviderError { code: ErrorClass, message: String, retryable: bool },
}

// ---- router ---------------------------------------------------------------
pub struct Router {
    chains: Vec<Chain>,                                  // typed, ordered
    rate_limits: RateLimitTracker,                       // from observed headers
    cost_policy: CostPolicy,
    quality_policy: QualityPolicy,                       // from models.dev benchmarks
}

pub struct Chain {
    pub name: String,
    pub primary: ModelRef,
    pub fallbacks: Vec<ModelRef>,
    pub retry: RetryPolicy,                              // typed per ErrorClass
    pub on_context_overflow: ContextOverflowPolicy,
    pub provider_routing: ProviderRoutingPolicy {
        precision: Precision::Exact,                     // default: refuse quantized
        allow_quantization: false,
        ...
    },
}

pub enum ErrorClass {
    RateLimit, ContextWindowExceeded, ContentPolicy,
    Auth, Timeout, InternalServer, NetworkTransient,
}

// ---- cost -----------------------------------------------------------------
pub struct CallCost {
    pub input_tokens: u32,
    pub cache_read_tokens: u32,
    pub cache_write_tokens: u32,
    pub output_tokens: u32,
    pub reasoning_tokens: u32,
    pub usd: f64,                                        // computed from models.dev pricing
    pub breakdown: CostBreakdown,
}

// ---- top-level call -------------------------------------------------------
pub struct Client { router: Router, registry: ToolRegistry, auths: AuthStore }

impl Client {
    pub async fn complete(&self, call: Call) -> Result<Completion>;
    pub async fn stream(&self, call: Call) -> Result<stream::EventStream>;
}
```

Notes on the sketch:
- The `Provider` trait is parameterized by a `CapabilitySet`; cross-provider request fields go through typed `ProviderExtensions<Caps>` so a request that uses `cache_control` against an `Implicit`-caching provider produces a typed *warning at build time* (rejected at `build_request`) instead of being silently dropped.
- `ToolRegistry::project` returns a `Projected` that the agent loop attaches to the call. `ProjectionMode::CodeExecution` returns a single shim tool + a synthesized module manifest, not N tool schemas. The same registry can serve `Inline` for older models.
- `Event` is the union — never lose tool-input deltas (LiteLLM #20711), parallel-call ordering (#21331), or reasoning content. `ToolResult { provider_executed }` makes Mastra-style merging trivial.
- `SubscriptionBackend::policy_status` makes the Anthropic situation legible. The default config does *not* enable `ClaudeCodeOAuth`; users opt in with eyes open.
- Router is plain Rust; serializable to TOML for users who want config files, but the *source of truth* is typed.

---

## 11. Citations

LiteLLM
- https://docs.litellm.ai/docs/routing
- https://docs.litellm.ai/docs/router_architecture
- https://docs.litellm.ai/docs/proxy/reliability
- https://docs.litellm.ai/docs/completion/prompt_caching
- https://docs.litellm.ai/blog/security-update-march-2026
- https://docs.litellm.ai/blog/security-hardening-april-2026
- https://github.com/BerriAI/litellm/issues/6345
- https://github.com/BerriAI/litellm/issues/14293
- https://github.com/BerriAI/litellm/issues/15299
- https://github.com/BerriAI/litellm/issues/15943
- https://github.com/BerriAI/litellm/issues/17246
- https://github.com/BerriAI/litellm/issues/19744
- https://github.com/BerriAI/litellm/issues/20711
- https://github.com/BerriAI/litellm/issues/21041
- https://github.com/BerriAI/litellm/issues/21090
- https://github.com/BerriAI/litellm/issues/21331
- https://github.com/BerriAI/litellm/issues/23247
- https://github.com/BerriAI/litellm/issues/25134

Vercel AI SDK
- https://ai-sdk.dev/docs/ai-sdk-core/tools-and-tool-calling
- https://ai-sdk.dev/docs/advanced/stopping-streams
- https://vercel.com/blog/ai-sdk-6
- https://vercel.com/blog/ai-sdk-5
- https://github.com/OpenRouterTeam/ai-sdk-provider/issues/35

Anthropic / Claude Code
- https://platform.claude.com/docs/en/build-with-claude/prompt-caching
- https://code.claude.com/docs/en/authentication
- https://code.claude.com/docs/en/legal-and-compliance
- https://support.claude.com/en/articles/11145838-use-claude-code-with-your-pro-or-max-plan
- https://www.anthropic.com/engineering/code-execution-with-mcp
- https://www.anthropic.com/engineering/advanced-tool-use
- https://www.theregister.com/2026/02/20/anthropic_clarifies_ban_third_party_claude_access/
- https://venturebeat.com/technology/anthropic-cracks-down-on-unauthorized-claude-usage-by-third-party-harnesses
- https://news.ycombinator.com/item?id=47069299
- https://news.ycombinator.com/item?id=46549823
- https://daveswift.com/claude-oauth-update/
- https://gist.github.com/shubcodes/3c9c7ff813715aa47018bf22e7cf8cb5

OpenAI / Codex
- https://developers.openai.com/codex/auth
- https://help.openai.com/en/articles/11369540-using-codex-with-your-chatgpt-plan
- https://github.com/openai/codex/issues/17265
- https://github.com/openai/codex/issues/4278
- https://github.com/numman-ali/opencode-openai-codex-auth
- https://community.openai.com/t/best-practice-for-clientid-when-using-codex-oauth/1371778
- https://openai.github.io/openai-agents-python/models/litellm/
- https://github.com/openai/openai-agents-python/issues/1257

Google / Gemini
- https://ai.google.dev/gemini-api/docs/oauth
- https://github.com/google-gemini/gemini-cli/issues/19970
- https://github.com/google-gemini/gemini-cli/issues/21866
- https://syntackle.com/blog/google-gemini-ai-subscription-with-opencode/

GitHub Copilot
- https://github.blog/changelog/2026-01-16-github-copilot-now-supports-opencode/
- https://github.com/ericc-ch/copilot-api
- https://github.com/caozhiyuan/copilot-api
- https://github.com/templarsco/opencode-copilot-bridge
- https://aider.chat/docs/llms/github.html
- https://github.com/orgs/community/discussions/160013
- https://github.com/orgs/community/discussions/130825

Rust ecosystem
- https://github.com/0xPlaygrounds/rig
- https://github.com/jeremychone/rust-genai
- https://github.com/jeremychone/rust-genai/issues/24
- https://github.com/64bit/async-openai
- https://github.com/graniet/llm
- https://github.com/avivsinai/litellm-rust
- https://swiftide.rs/
- https://github.com/sobelio/llm-chain
- https://crates.io/crates/claude-code-mux

OpenRouter / quality drift
- https://openrouter.ai/announcements/provider-variance-introducing-exacto
- https://openrouter.ai/docs/guides/routing/provider-selection
- https://github.com/RooCodeInc/Roo-Code/issues/11325
- https://github.com/QwenLM/qwen-code/pull/348
- https://news.ycombinator.com/item?id=47700972
- https://news.ycombinator.com/item?id=47563884

JIT tool projection / MCP
- https://github.com/NousResearch/hermes-agent/issues/4379
- https://github.com/modelcontextprotocol/modelcontextprotocol/issues/1576
- https://www.anthropic.com/engineering/code-execution-with-mcp
- https://www.philschmid.de/mcp-cli
- https://layered.dev/mcp-tool-schema-bloat-the-hidden-token-tax-and-how-to-fix-it/
- https://onlycli.github.io/OnlyCLI/blog/mcp-token-cost-benchmark/

Cost-aware routing / research
- https://github.com/lm-sys/RouteLLM
- https://www.lmsys.org/blog/2024-07-01-routellm/
- https://arxiv.org/pdf/2510.08439 (xRouter)
- https://arxiv.org/html/2508.12491 (cost-aware contrastive routing)
- https://research.ibm.com/blog/LLM-routers

LiteLLM supply-chain attack
- https://securitylabs.datadoghq.com/articles/litellm-compromised-pypi-teampcp-supply-chain-campaign/
- https://snyk.io/blog/poisoned-security-scanner-backdooring-litellm/
- https://cycode.com/blog/lite-llm-supply-chain-attack/
- https://www.trendmicro.com/en_us/research/26/c/inside-litellm-supply-chain-compromise.html
- https://www.infoq.com/news/2026/03/litellm-supply-chain-attack/

Provider rate-limits / OpenAI-compat
- https://inference-docs.cerebras.ai/support/rate-limits
- https://ai-sdk.dev/providers/openai-compatible-providers/nim
- https://docs.nvidia.com/nim/large-language-models/latest/api-reference.html

opencode / cline / models.dev
- https://opencode.ai/docs/providers/
- https://opencode.ai/docs/models/
- https://deepwiki.com/sst/opencode/3.3-provider-and-model-configuration
- https://deepwiki.com/anomalyco/opencode/2.4-ai-provider-and-model-management

Streaming / divergence
- https://medium.com/percolation-labs/comparing-the-streaming-response-structure-for-different-llm-apis-2b8645028b41
- https://amitkoth.com/claude-vertex-ai-vs-native-api/
- https://docs.aws.amazon.com/bedrock/latest/userguide/prompt-caching.html

Portkey / Helicone comparisons
- https://www.truefoundry.com/blog/truefoundry-vs-portkey-vs-helicone-enterprise-ai-gateway-comparison
- https://www.helicone.ai/blog/top-llm-gateways-comparison-2025

Mastra
- https://mastra.ai/docs/streaming/tool-streaming
- https://mastra.ai/blog/announcing-mastra-improved-agent-orchestration-ai-sdk-v5-support
