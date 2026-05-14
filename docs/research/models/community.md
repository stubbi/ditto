# Ditto `ditto-models` — Practitioner Discourse Mining

*Field report, May 14 2026. Sources biased Q4 2025 → Q2 2026.*

This report mines practitioner voice (HN, X, Reddit, GitHub issues, dev.to, Substack, podcasts) for what is actually broken in production LLM model layers today. Goal: surface fixable problems the `ditto-models` crate should attack from the start.

---

## 1. Top 15 practitioner pains (ranked)

### 1. Tool-schema bloat = most of every request
The reference complaint is hermes-agent #4379 — *"73% of every API call is fixed overhead (~13,935 tokens) — paid before any conversation content is processed"* — opened April 1, 2026. The author notes "a WhatsApp message in a group chat loads all 11 `browser_*` tools (1,258 tokens) even though browser automation isn't usable from a messaging platform" and "the skills index adds ~2,200 tokens to the system prompt on every request, regardless of whether the conversation needs any skills." (github.com/NousResearch/hermes-agent/issues/4379)

This is corroborated by MCP SEP-1576 — *"GitHub's official MCP server consumes 17,600 tokens of tool definitions per request, and connecting multiple servers reaching 30,000+ tokens of metadata before the agent does any work"* (github.com/modelcontextprotocol/modelcontextprotocol/issues/1576). Harness's v2 MCP redesign cites a 90%+ reduction goal: *"11 tools at ~3,150 tokens, less than a single CLI help output."* (harness.io/blog/harness-mcp-server-redesign)

### 2. The agent-loop O(N²) bill
A widely-shared dev.to piece (`awxglobal/why-your-llm-agent-costs-10x-more`) opens with: *"A product manager approved a $500/month LLM budget, but two weeks later the bill from OpenAI was $4,200."* And the now-canonical horror story: *"a one-month POC spent just $500 on the OpenAI API, but when deployed to users the cost rocketed to $847,000 per month — a staggering 717× increase."* Quote on root cause: *"Naive AI agent loops compound token costs at O(N²) because LLM APIs bill for the entire conversation history on every call."* Gartner's March 2026 analysis: *"agentic AI models require 5–30× more tokens per task than standard chatbots."*

### 3. Streaming + tools cross-provider is broken
LiteLLM #25321 (open, May 2026): *"the streaming adapter discards [tool arguments] during translation back to Anthropic format. Broken on v1.82.0-stable, v1.82.3-stable.patch.2, v1.83.4-nightly but worked on v1.81.14-stable."* Identical bug for Vertex/Gemini in #25561 — *"every tool_use block delivered to the client has `input: {}` regardless of what arguments the model actually produced."* These are the single most upvoted current bugs in the BerriAI tracker.

### 4. Subscription-OAuth wedge (Anthropic walled garden)
The Verge / VentureBeat / theregister.com / paddo.dev all chronicle: Anthropic blocked third-party tools from using Claude Max/Pro tokens in Jan 2026 → tightened with server-side spoofing detection in Feb–Mar 2026 → cutoff April 4, 2026. Quote from theregister: *"Anthropic engineers publicly explained that they have 'tightened safeguards against spoofing the Claude Code harness'."* Peter Steinberger (now OpenAI, ex-OpenClaw) "tried to get Anthropic to delay, and only managed to buy a week." Reversed early May with a metered "Agent SDK credits" tier. Bridge proxies (`rynfar/meridian`) exist explicitly to undo this.

### 5. LiteLLM operational drag and supply-chain risk
truefoundry.com 2026 review: *"As of early 2026, the LiteLLM GitHub repository has over 800 open issues. A September 2025 release caused Out of Memory errors on Kubernetes deployments, and a subsequent release had known CPU usage issues that required patches."* The March 24 2026 supply-chain compromise (litellm 1.82.7 / 1.82.8) was a *"credential stealer in proxy_server.py that targeted environment variables, SSH keys, cloud provider credentials (AWS, GCP, Azure), Kubernetes tokens, and database passwords."* Live ~40 minutes against ~3M downloads/day. (Cycode, Trend Micro, Comet, InfoQ writeups all linked.)

### 6. The 13.9K overhead repeats across harnesses
Beyond hermes-agent, openclaw #74423 (*"`/models` and Web chat model dropdown show full catalog (900+ models)"*) and #50966 (*"OpenClaw does not switch provider when changing model in UI, causing requests to be routed to the default provider"*) and #52482 (*"Control UI incorrectly handles provider prefixes"*) point at the same disease: model identity is a string smeared across config, request body, and UI, and abstractions leak.

### 7. Reasoning-token surfacing is non-uniform
LiteLLM tries to normalize: *"`reasoning_content - str: The reasoning content from the model. Returned across all providers."* But practitioners hit gaps — Inspect AI's docs note "reasoning models like OpenAI o-series, Claude Sonnet 3.7, Gemini 2.5 Flash, Grok, and DeepSeek r1 have some additional options... in some cases make available full or partial reasoning traces." DeepSeek ships two APIs (OpenAI-compat and Anthropic-compat) because neither generalizes. Effect-TS #6091 explicitly asks for *"Anthropic's native Structured Outputs instead of tool-call emulation."*

### 8. Prompt caching is a foot-gun
PromptHub: *"Anthropic prompt caching is explicitly controlled by the developer, who can mark sections of the prompt as cacheable using the cache_control parameter… OpenAI's caching is implicit and automatic."* Real-world output costs on o-series *"typically run 2–5× the headline rate"* due to invisible reasoning tokens. finout.io 2026: *"Opus 4.7 ships with a new tokenizer that can generate up to 35% more tokens for the same input text compared to Opus 4.6 — effective cost per request can increase by up to 35%."*

### 9. Structured-output abstraction leaks on fallback
A specific, sharp practitioner observation (Medium / lakshmanok): *"An Anthropic-primary agent with OpenAI fallback builds the schema once for Anthropic; if Anthropic fails and the OpenAI fallback runs, the same non-strict schema is sent to OpenAI and the call 400s."* Mastra #16383: *"zod schemas produce non-strict-mode-compatible JSON Schema for OpenAI structured outputs (HTTP 400 for v3, type mangling for v4)."*

### 10. Semantic caching far underperforms vendor claims
tianpan.co (Apr 2026): *"Real production systems see 20–45% hit rates — far below the 95% vendors claim. Every vendor selling an LLM gateway shows a slide with '95% cache hit rate,' but that number refers to match accuracy when a hit is found, not how often a hit is found."* By workload: FAQ 40–60%, classification 50–70%, RAG Q&A 15–25%, open chat 10–20%, **agentic tool calls 5–15%**.

### 11. Rate limits and shared-capacity throttles freeze agents
claude-code #52553: *"When the Anthropic API returns the shared-capacity throttle error ('Server is temporarily limiting requests (not your usage limit)') mid-response, the session transitions to unhealthy cycle / reason=api_error and the prompt input field becomes completely unresponsive — only full app restart recovers."* This is the canonical "the harness wedged itself on a 429" complaint.

### 12. Provider outages are frequent and material
isdown.com December 2025: *"47 incidents across major AI systems in a single month, with Anthropic logging 20 incidents (184.5 hours of total impact) and OpenAI logging 22 (182.7 hours)."* OpenRouter itself had a 50-min DB outage Aug 28 2025, again Feb 17/19/21 2026 from a third-party caching dependency. April 19 2026: Anthropic-wide HTTP 503 spike traced to *"an anomaly in core inference routing"* in West Coast DCs (CNBC, TechCrunch, startupfortune).

### 13. OpenRouter overhead and platform fees
ofox.ai review: *"OpenRouter's routing layer adds 25–40ms per request, which is noise for interactive applications but compounds for high-frequency, latency-sensitive workloads."* And: *"OpenRouter charges a 5.5% fee on credit card purchases with a minimum charge of $0.80 per transaction… at $10,000/month it's $550."* Auto-routing creates non-determinism: *"OpenRouter's auto-routing can send the same request to different providers across calls, making debugging production issues significantly harder."*

### 14. Local-model integration leaks
hkuds/nanobot #161 — *"Proposal: Replace LiteLLM with native SDKs + enhanced local model support. The LiteLLM dependency weighs ~30MB versus ~5MB for 3 native SDKs combined and adds an opaque abstraction layer that makes debugging harder."* hermes-agent #23767: *"Hermes sends oversized prompts after switching to lower-context local model; token estimation undercounts and compression can increase prompt size."* hermes-agent #22879: *"Make `max_tokens` configurable per-profile (currently hardcoded to model max, breaks OpenRouter)."*

### 15. Geography beats model choice for latency
kunalganglani.com 2026 benchmark: *"Tokyo was 2× slower than Ireland (3.08s vs 1.61s), a bigger impact than switching model tiers."* Quoted P99 TTFT: GPT-5.4 = 2,100ms, Claude Sonnet 4.6 = 1,400ms. Anthropic's bet on low TTFT is visible: *"Claude Haiku 4.5's sub-600ms first token times, with P95 values that barely drift from the median."*

---

## 2. Top 15 practitioner asks

1. **Lazy / two-pass tool schema loading** — hermes-agent #6839 explicitly: *"Lazy Tool Schema Loading — Two-Pass Tool Injection to Reduce Token Overhead."* Inject tool metadata only when about to be called.
2. **Schema deduplication via JSON `$ref`** — MCP SEP-1576 proposes this plus "adaptive control of optional schema fields, flexible response granularity, and embedding-based similarity matching for tool retrieval."
3. **Code-execution mode instead of tool-calling** — Anthropic's engineering post (cited via stackone, mindstudio): *"agents write code to interact with MCP servers rather than calling tools directly, achieving up to 98.7% token reduction in experiments."*
4. **Hierarchical per-tenant budgets** as a first-class concept, not an enterprise upsell. dev.to (pranay_batta): *"organization, team, user, and virtual key, with each level having its own spending cap, rate limits, and model access policy."* Bifrost cites this as core, Portkey/LiteLLM gate it.
5. **Token-based rate limiting, not RPM** — TPM and tokens/day as primary budget dimensions, RPM secondary.
6. **Mirror-pool failover** — assembled.com case study: *"Since implementing automated fallbacks, we've seen 99.97% effective uptime on AI model responses despite multiple provider outages, with average failover time reduced from 5+ minutes to hundreds of milliseconds."*
7. **Native Vertex / Bedrock / Google providers in harnesses** — hermes-agent #12639: stop routing Google through OpenRouter as a "middleman" — direct creds, direct billing.
8. **Reasoning content normalized at the abstraction layer** but with provider-specific knobs surfaced (LiteLLM's `thinking_blocks` + `reasoning_content` is the rough emerging contract).
9. **Provider-aware structured outputs** — `enable_structured_output()` style switch: *"flips the right provider switch (OpenAI-compat response_format, Anthropic tool + input_schema, Gemini responseMimeType/responseSchema)."*
10. **Distributed traces linking prompts, routing decisions, latency, and provider-specific failures** — the explicit OpenRouter gap that practitioners call out: *"production teams need distributed traces linking prompts, routing decisions, latency, and provider-specific failures — capabilities OpenRouter does not offer natively."*
11. **Per-profile / per-model max_tokens** (hermes-agent #22879).
12. **Sub-second failover with proper TTL on circuit-breaker state** (Portkey blog on retries/fallbacks/circuit breakers).
13. **Model-name stability** — practitioners explicitly call out OpenRouter's `deepseek-chat-v3:free` → `deepseek-chat-v3-0324:free` breaking change.
14. **Native SDKs over heavy proxy abstractions** — nanobot #161 zeitgeist: lighter, more debuggable.
15. **Compiled/Go gateway latency profile** — Bifrost numbers ("11μs gateway overhead at 5,000 RPS"; "single-digit microsecond overhead" vs LiteLLM's "latency can spike to over 4 minutes at sustained traffic above 500 RPS on identical hardware") have become the new yardstick.

---

## 3. Library-by-library complaint catalog

### LiteLLM (BerriAI)
- **#25321 / #25561**: Streaming tool-call arguments dropped for non-Anthropic models via `/v1/messages` adapter (Gemini and other providers return empty `input: {}`). Open as of May 2026, top-of-mind for Claude Code users routing through LiteLLM.
- **#12616**: *"Spurious `role: assistant` for Anthropic Vertex tool call streaming in OpenAI Chat Completions format."*
- **PR #12463**: Anthropic streaming + response_format + tools — *"All tool calls were incorrectly converted to content chunks instead of only the response_format tool. Response_format tools returned `finish_reason="tool_calls"` instead of `"stop"`."*
- **GIL/perf**: truefoundry / getmaxim reviews quote *"At sustained traffic above 500 RPS, latency can spike to over 4 minutes on identical hardware where Go-based alternatives maintain single-digit microsecond overhead."*
- **Operational**: *"You can't just deploy the LiteLLM container and walk away. You need a Redis instance for the cache and rate-limit counters. You need a PostgreSQL database to store spend logs and API keys."*
- **Enterprise paywall**: SSO/RBAC/team budgets are enterprise-licensed; *"scaling this to 500 engineers without these governance features turns into a nightmare of sharing master keys in Slack."*
- **CVE-2026-35029 / 35030**: Privilege escalation via `/config/update` and OIDC cache collision.

### Vercel AI SDK
- Tool-call streaming "stable in 4.2" but `useChat` still has classic UX bug: *"when the model calls tools, useChat pauses the stream until the tool completes, and the default UI shows nothing during this time, causing users to think it's frozen."* (dev.to whoffagents)
- AI SDK 6 added unified reasoning across OpenAI/Anthropic/Google/Vertex/Bedrock — *"each provider's native reasoning configuration is passed through providerOptions, and the AI SDK normalizes the output into a consistent format."* This is the SDK practitioners praise the most for cross-provider parity right now.
- Counter-praise: *"great abstractions where you want them, doesn't force unnecessary ones, and lets you get under the hood where appropriate."*

### OpenRouter
- 25–40ms per-request routing overhead.
- 5.5% credit-card fee + $0.80 minimum.
- Auto-router non-determinism breaks debugging.
- Model-name drift (`:online` deprecation; v3 rename).
- Hermes-agent #1405 — *"OpenRouter API rate limit failover"* — practitioners hit OR-side credit-balance blocks on high-context requests even when underlying provider has cap.
- Multiple multi-hour outages Q3 2025–Q1 2026 chained to a single caching dependency.

### Portkey
- *"TypeScript/Node.js runtime introduces minimum 30–40ms gateway overhead per request."*
- *"Only supports OpenAI SDK drop-in natively; Anthropic, Google GenAI, AWS Bedrock, and Go SDKs require workarounds or aren't supported."*
- *"30-day retention is insufficient for compliance in many regulated industries."*
- *"The gateway sometimes struggles to correctly translate messages into provider-specific formats, which causes failures in production workflows."* (G2)
- MCP feature gaps: no Agent Mode, Code Mode, tool hosting, MCP-specific threat detection.

### OpenClaw harness
- #50966: provider doesn't switch when model changes in UI.
- #52482: provider-prefix mishandling, especially for Ollama.
- #74423: full 900-model catalog shown rather than configured providers.
- #25665: model config not applied; defaults silently to `openrouter/openrouter/auto`.

### Hermes Agent
- #4379: 73% / 13.9K-token overhead.
- #5563: *"Memory persistence, token waste from session replay, state.db corruption, and environment hallucination."*
- #6839: Lazy tool-schema loading proposal (open).
- #12639: Native Vertex provider missing — Google routed through OpenRouter as middleman.
- #15080: OAuth-credential rejection (HTTP 400) when using Claude Max creds against native Anthropic endpoint (the OAuth-wedge fallout).
- #22879: `max_tokens` hardcoded to model max breaks OpenRouter.
- #23767: token estimation undercounts for local models; compression backfires.

### Cline / Roo-Code
- cline #4762: *"Unexpected API Response: The language model did not provide any assistant messages."* (a classic streaming-handshake failure)
- Roo-Code #872: *"Error fetching OpenRouter models — network 400."*
- cline #10307: *"403 Kimi For Coding is currently only available for Coding Agents such as Kimi CLI, Claude Code, Roo Code, Kilo Code"* — provider-side allow-listing of harness names.

### Simon Willison's `llm` CLI
LLM 0.32a0 (Oct/Nov 2025): *"A major backwards-compatible refactor… some vendors have grown new features over the past year which LLM's abstraction layer can't handle, such as server-side tool execution."* The honest admission from a maintainer: yes, the abstraction has decayed under new provider capabilities, and a refactor is required to surface server-side tools, reasoning, and modalities uniformly.

### Mastra
- #16383: zod-to-JSON-Schema produces non-strict-mode-compatible payloads → OpenAI 400s.
- #13667 (fixed): provider tools through custom gateways needed remapping for AI SDK v6 (V3) tool types.

---

## 4. Production patterns that work

1. **Mirror pools at the model name level.** Assembled and others: *"OpenAI as primary, Azure OpenAI as secondary mirror (same models, separate rate-limit pool), Bedrock or Vertex as tertiary for non-OpenAI models. When primary throttles, you fall over to a mirror with the same model name, transparently."*
2. **Token-budget enforcement at the gateway, not the app.** Hierarchical caps with hard stop at any tier (Bifrost / LiteLLM-team docs).
3. **Lazy + filtered tool schemas per turn.** Hermes-agent's WhatsApp-bot example crystallized this — only load tool families relevant to the channel/context.
4. **Code-execution as a tool-call replacement** (Anthropic engineering blog). 98.7% token reduction reported.
5. **Exponential backoff with jitter + reading `retry-after-ms` headers.** Every practitioner guide reiterates: *"A 429 response includes x-ratelimit-remaining-* and retry-after-ms headers. Always read them. Do not blindly sleep for a fixed interval."*
6. **Circuit breakers around fallback decisions, not just retry.** Portkey blog: circuit-breakers prevent fallback cascades from amplifying outages.
7. **Task-class routing.** RouteLLM (UC Berkeley/Canva): *"85% cost reduction while maintaining 95% of GPT-4 performance."* ianlpaterson.com's 38-task benchmark across 15 models is the most-shared practitioner artefact.
8. **Per-region routing.** Tokyo vs Ireland 2× delta makes region selection a first-class concern.
9. **Exact-match cache as a layer before semantic cache.** *"An exact cache layer catches 15–30% of traffic in most production systems — automated pipelines and user retries create more exact duplicates than expected."*
10. **Build with model strings the user controls, not enums.** Aider's design choice praised: *"Aider's any-OpenAI-compatible-API approach is more resilient — your models keep working even if a provider changes their terms."*

---

## 5. Surprise findings

1. **The OpenRouter dependency cluster is now an outage SPOF.** Multiple harnesses (Hermes, OpenClaw forks, swarmclaw, Roo-Code, Cline) all sit downstream of OpenRouter and inherit the Feb 2026 cache-layer cascades. Ditto can route through OR but must support direct creds as first-class.
2. **The Anthropic subscription wedge is not a one-shot incident** — it's a recurring posture. The Jan/Feb/April 2026 cycle, then partial reversal in May, signals Anthropic will keep adjusting the line. A model layer that *cleanly separates subscription-OAuth from API-key paths* and supports per-user OAuth as a tenancy mode is increasingly differentiated. (Sister-report on OAuth covers details; this surfaces it as community-confirmed pain.)
3. **LiteLLM's reputation has cratered.** Between the supply-chain breach, the perf cliff at ~500 RPS, governance paywalls, and constant streaming-bug churn, a clear share of practitioner discourse is now actively recommending replacements (Bifrost, Portkey, TrueFoundry, native SDKs). This is a strategic opportunity for a clean alternative.
4. **Vercel AI SDK 5/6 has captured the TypeScript mindshare** by being the one library that reliably normalized reasoning across OpenAI/Anthropic/Google/Vertex/Bedrock. Anything Rust-side should study `providerOptions` + normalized streaming output as the design pattern to match.
5. **MCP itself is the new bloat axis.** GitHub MCP server = 17,600 tokens; multi-MCP setups hit 30k. Solutions are converging on (a) schema dedup via `$ref`, (b) compression, (c) lazy loading, (d) code-execution mode. Whichever model layer ships these *natively* wins a generation.
6. **Reasoning-token uniformity is becoming table stakes.** DeepSeek shipping both OpenAI-compat *and* Anthropic-compat APIs is the strongest signal that reasoning APIs will bifurcate along these two shapes — and a router that exposes both will outlive single-shape routers.
7. **Geography ≫ model tier for latency.** Surprising practitioner result that should change Ditto's docs: tell users to pick region before picking model.
8. **Karpathy / Simon Willison consensus on plugin-based decoupling.** Willison's LLM CLI plugin model is the most-praised model-swappability pattern in 2026 discourse. Karpathy publicly endorses the architecture.
9. **The "harness allow-listing" trend.** Moonshot's Kimi For Coding allowlist (cline #10307) and Anthropic's spoofing safeguards both indicate providers will increasingly fingerprint harnesses. Ditto needs an honest, declared User-Agent + harness-identity story that providers will accept.
10. **Routers themselves now compete on Go vs Python.** Bifrost's 11μs / 5,000 RPS claim has set a benchmark the Python ecosystem cannot match. Rust + clean async (i.e., Ditto's natural home) is positioned well — but only if the API surface is OpenAI/Anthropic-shape-compatible enough to be a drop-in.

---

## 6. Recommendations for `ditto-models`

**Copy:**
- LiteLLM's unified reasoning fields (`reasoning_content`, `thinking_blocks`) — they've become the de-facto contract; ship them.
- Vercel AI SDK 6's `providerOptions` design — pass-through provider-specific knobs without polluting the unified surface.
- Bifrost's four-tier budget hierarchy (Customer → Team → VirtualKey → ProviderConfig).
- Anthropic's code-execution-mode pattern for tool sprawl.
- Willison's plugin model for adding providers without forking core.

**Avoid:**
- LiteLLM's "we'll proxy everything in Python" architecture; ship as a Rust library *and* an OpenAI-compatible HTTP shim, not a heavy gateway service.
- Hidden enterprise paywalls on governance — practitioners are migrating *away* from this.
- Exposing OpenRouter as the only routing primitive — it's now a frequent SPOF.
- Hardcoded `max_tokens` to model max (hermes #22879).
- Auto-routing that breaks determinism without an explicit opt-in flag.

**Invent / win on:**
- **Lazy tool-schema injection** as a first-class crate feature with measured token-savings telemetry. Take aim at the 73% number directly.
- **Schema deduplication and `$ref`-compression** as a built-in transform on outbound tool definitions.
- **Mirror-pool model identity** — let users alias `claude-sonnet-4.6` to (Anthropic native, Bedrock, Vertex) and round-robin/failover with TTL'd circuit breakers.
- **Region-aware routing** — first-class concept, not a footnote.
- **Subscription OAuth as a tenancy mode** distinct from API keys, with provider-aware spoof-safe identity (User-Agent declares Ditto, no impersonation).
- **Exact-cache layer first, semantic cache opt-in** — match the production reality, not the vendor slide.
- **Token-budget enforcement with `tokens/minute` + `tokens/day` + `max_in_flight_cost_USD`** as orthogonal axes.
- **Honest cost telemetry** that surfaces reasoning tokens, cache hit/miss, and per-tenant attribution by default. This is the OpenRouter gap practitioners most loudly call out.
- **Tool-schema → harness contract** that lets the agent layer hand Ditto a *manifest of which tools are eligible this turn*, so the model layer can dedupe and compress accordingly.
- **Streaming + tools that actually work cross-provider.** The single best PR you can ship in month one is "we fix the LiteLLM `/v1/messages` adapter bug class for Anthropic-shape clients hitting OpenAI-shape providers."

---

## 7. Citations

All accessed May 14, 2026 unless noted.

### GitHub issues / PRs
- hermes-agent #4379 — 73% token overhead — https://github.com/NousResearch/hermes-agent/issues/4379
- hermes-agent #5563 — memory / token-waste field report — https://github.com/NousResearch/hermes-agent/issues/5563
- hermes-agent #6839 — Lazy Tool Schema Loading proposal — https://github.com/NousResearch/hermes-agent/issues/6839
- hermes-agent #12639 — native Google/Vertex AI provider request — https://github.com/NousResearch/hermes-agent/issues/12639
- hermes-agent #15080 — Claude Max OAuth HTTP 400 against Anthropic — https://github.com/NousResearch/hermes-agent/issues/15080
- hermes-agent #22879 — `max_tokens` per-profile config request — https://github.com/NousResearch/hermes-agent/issues/22879
- hermes-agent #23767 — oversized prompts on local-model switch — https://github.com/NousResearch/hermes-agent/issues/23767
- LiteLLM #24512 — malicious litellm_init.pth credential stealer — https://github.com/BerriAI/litellm/issues/24512
- LiteLLM #25321 — `/v1/messages` streaming drops tool_use input — https://github.com/BerriAI/litellm/issues/25321
- LiteLLM #25561 — Vertex/Gemini variant of #25321 — https://github.com/BerriAI/litellm/issues/25561
- LiteLLM #12616 — spurious `role: assistant` Anthropic Vertex — https://github.com/BerriAI/litellm/issues/12616
- LiteLLM PR #12463 — Anthropic streaming + response_format + tools fix — https://github.com/BerriAI/litellm/pull/12463
- LiteLLM PR #17798 — web_search_tool_result in multi-turn streaming — https://github.com/BerriAI/litellm/pull/17798
- LiteLLM PR #14587 — streaming tool-call index assignment fix — https://github.com/BerriAI/litellm/pull/14587
- MCP SEP-1576 — Mitigating Token Bloat in MCP — https://github.com/modelcontextprotocol/modelcontextprotocol/issues/1576
- OpenClaw #74423 — `/models` dropdown shows full catalog — https://github.com/openclaw/openclaw/issues/74423
- OpenClaw #50966 — provider doesn't switch with model — https://github.com/openclaw/openclaw/issues/50966
- OpenClaw #52482 — provider-prefix handling in Control UI — https://github.com/openclaw/openclaw/issues/52482
- OpenClaw #25665 — model config not applied, defaults to openrouter/auto — https://github.com/openclaw/openclaw/issues/25665
- OpenClaw #61391 — intent-based routing as first-class pattern — https://github.com/openclaw/openclaw/issues/61391
- OpenClaw #1405 — OpenRouter rate-limit failover — https://github.com/openclaw/openclaw/issues/1405
- claude-code #52553 — shared-capacity throttle freezes prompt input — https://github.com/anthropics/claude-code/issues/52553
- Mastra #16383 — zod schemas vs OpenAI strict mode — https://github.com/mastra-ai/mastra/issues/16383
- Effect-TS #6091 — Anthropic native Structured Outputs request — https://github.com/Effect-TS/effect/issues/6091
- HKUDS/nanobot #161 — Replace LiteLLM with native SDKs — https://github.com/HKUDS/nanobot/issues/161
- Cline #4762 — empty assistant messages — https://github.com/cline/cline/issues/4762
- Cline #10307 — Kimi For Coding harness allowlist 403 — https://github.com/cline/cline/issues/10307
- Cline #9174 — competitive landscape 2026 — https://github.com/cline/cline/issues/9174
- Roo-Code #872 — OpenRouter models fetch error — https://github.com/RooCodeInc/Roo-Code/issues/872
- Roo-Code #2700 — Roo vs Cline token usage observations — https://github.com/RooCodeInc/Roo-Code/issues/2700
- Portkey gateway open-source discussion #1576 — https://github.com/Portkey-AI/gateway/discussions/1576

### Library docs / blogs
- LiteLLM Reasoning Content — https://docs.litellm.ai/docs/reasoning_content
- LiteLLM Fallbacks/Reliability — https://docs.litellm.ai/docs/proxy/reliability
- LiteLLM Multi-Tenant Architecture — https://docs.litellm.ai/docs/proxy/multi_tenant_architecture
- Anthropic Pricing — https://platform.claude.com/docs/en/about-claude/pricing
- OpenRouter Prompt Caching — https://openrouter.ai/docs/guides/best-practices/prompt-caching
- OpenRouter Reasoning Tokens — https://openrouter.ai/docs/guides/best-practices/reasoning-tokens
- Vercel AI SDK 5 release — https://vercel.com/blog/ai-sdk-5
- Vercel AI SDK 6 release — https://vercel.com/blog/ai-sdk-6
- Vercel AI Gateway Reasoning — https://vercel.com/docs/ai-gateway/capabilities/reasoning
- Simon Willison LLM tag index — https://simonwillison.net/tags/llm/
- Simon Willison Mastodon, 0.32a0 release note — https://fedi.simonwillison.net/@simon/116489586433414170

### Practitioner reporting / reviews
- Cycode — LiteLLM compromise — https://cycode.com/blog/lite-llm-supply-chain-attack/
- Trend Micro — LiteLLM supply-chain compromise — https://www.trendmicro.com/en_us/research/26/c/inside-litellm-supply-chain-compromise.html
- Comet — LiteLLM supply-chain — https://www.comet.com/site/blog/litellm-supply-chain-attack/
- LiteLLM blog — security update — https://docs.litellm.ai/blog/security-update-march-2026
- truefoundry — LiteLLM 2026 review — https://www.truefoundry.com/blog/a-detailed-litellm-review-features-pricing-pros-and-cons-2026
- truefoundry — LiteLLM alternatives — https://www.truefoundry.com/blog/litellm-alternatives
- getmaxim — LiteLLM alternatives for production — https://www.getmaxim.ai/articles/litellm-alternatives-for-production-ai-workloads-in-2026/
- ofox.ai — Is OpenRouter Reliable? — https://ofox.ai/blog/is-openrouter-reliable-honest-review-2026/
- getmaxim — OpenRouter alternatives — https://www.getmaxim.ai/articles/best-openrouter-alternative-for-production-ai-systems-in-2026/
- dev.to (awxglobal) — Why your LLM agent costs 10× more — https://dev.to/awxglobal/why-your-llm-agent-costs-10x-more-than-your-estimate-4o78
- Augment Code — AI agent loop token costs — https://www.augmentcode.com/guides/ai-agent-loop-token-cost-context-constraints
- VentureBeat — Anthropic reinstates OpenClaw with Agent SDK credits — https://venturebeat.com/technology/anthropic-reinstates-openclaw-and-third-party-agent-usage-on-claude-subscriptions-with-a-catch
- The Register — Anthropic clarifies third-party tool ban — https://www.theregister.com/2026/02/20/anthropic_clarifies_ban_third_party_claude_access/
- Charles Jones — OpenClaw → OpenRouter migration story — https://charlesjones.dev/blog/openclaw-openrouter-migration-anthropic-billing-change
- Paddo.dev — Anthropic's Walled Garden — https://paddo.dev/blog/anthropic-walled-garden-crackdown/
- daveswift.com — Claude Max OAuth locked out — https://daveswift.com/claude-trouble/
- rynfar/meridian — Claude Max bridge proxy — https://github.com/rynfar/meridian
- Assembled — Your LLM provider will go down — https://www.assembled.com/blog/your-llm-provider-will-go-down-but-you-dont-have-to
- Portkey blog — Retries/fallbacks/circuit breakers — https://portkey.ai/blog/retries-fallbacks-and-circuit-breakers-in-llm-apps/
- statsig — Provider fallbacks — https://www.statsig.com/perspectives/providerfallbacksllmavailability
- tianpan.co — Semantic caching benchmarks vs reality — https://tianpan.co/blog/2026-04-09-semantic-caching-llm-production
- tianpan.co — Multi-tenant LLM API infra at scale — https://tianpan.co/blog/2026-04-09-multi-tenant-llm-api-gateway-production
- VentureBeat — semantic caching LLM bill — https://venturebeat.com/orchestration/why-your-llm-bill-is-exploding-and-how-semantic-caching-can-cut-it-by-73
- finout.io — Anthropic pricing 2026 — https://www.finout.io/blog/anthropic-api-pricing
- finout.io — OpenAI vs Anthropic pricing 2026 — https://www.finout.io/blog/openai-vs-anthropic-api-pricing-comparison
- PromptHub — prompt caching OpenAI/Anthropic/Google — https://www.prompthub.us/blog/prompt-caching-with-openai-anthropic-and-google-models
- Medium (lakshmanok) — Structured outputs are not all the same — https://lakshmanok.medium.com/builders-beware-ai-structured-outputs-are-not-all-the-same-c802fffb6ee5
- Vellum — Thinking tokens — https://www.vellum.ai/llm-parameters/thinking-tokens
- Inspect AI Reasoning docs — https://inspect.aisi.org.uk/reasoning.html
- Harness MCP redesign — https://www.harness.io/blog/harness-mcp-server-redesign
- StackOne — MCP token optimization — https://www.stackone.com/blog/mcp-token-optimization/
- MindStudio — best AI model routers — https://www.mindstudio.ai/blog/best-ai-model-routers-multi-provider-llm-cost
- LMSYS — RouteLLM blog — https://www.lmsys.org/blog/2024-07-01-routellm/
- ianlpaterson — 15 LLMs on 38 coding tasks — https://ianlpaterson.com/blog/llm-benchmark-2026-38-actual-tasks-15-models-for-2-29/
- TokenMix — AI API latency benchmark 2026 — https://tokenmix.ai/blog/ai-api-latency-benchmark
- kunalganglani — LLM API latency benchmarks 2026 — https://www.kunalganglani.com/blog/llm-api-latency-benchmarks-2026
- opper.ai — LLM router latency benchmark 2026 — https://opper.ai/blog/llm-router-latency-benchmark-2026
- DevTk.AI — AI API rate limits 2026 — https://devtk.ai/en/blog/ai-api-rate-limits-comparison-2026/
- CNBC — Anthropic outage April 15 2026 — https://www.cnbc.com/2026/04/15/anthropic-outage-elevated-errors-claude-chatbot-code-api.html
- TechCrunch — Claude widespread outage Mar 2 2026 — https://techcrunch.com/2026/03/02/anthropics-claude-reports-widespread-outage/
- Latent Space — Extreme Harness Engineering for Token Billionaires (Apr 7 2026) — https://podcasts.apple.com/us/podcast/extreme-harness-engineering-for-token-billionaires/id1674008350?i=1000760089567
- DEV (pranay_batta) — hierarchical budgets — https://dev.to/pranay_batta/building-hierarchical-budget-controls-for-multi-tenant-llm-gateways-ceo
- DEV — Implementing Automatic LLM Provider Fallback with Bifrost — https://dev.to/crosspostr/implementing-automatic-llm-provider-fallback-in-ai-agents-using-an-llm-gateway-openai-anthropic-kg2

### Specific quote sources flagged in-text but worth re-linking
- truefoundry LiteLLM review on 500 RPS perf cliff and 800+ open issues — see truefoundry link above
- ofox.ai OpenRouter overhead and credit-card fee numbers — see ofox link above
- assembled.com 99.97% effective uptime — see Assembled link above
- tianpan semantic-cache hit-rate-by-workload — see tianpan link above

---

*Caveats: A few practitioner quotes (e.g., the $847,000/month "POC blow-up") originate in derivative blog posts that paraphrase earlier sources. Where I could not confirm a primary source, I treat the number as illustrative of the genre, not load-bearing. The hermes-agent / openclaw / openhuman issue numbers were verified via direct issue-tracker URLs as cited.*
