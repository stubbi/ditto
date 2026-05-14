# Ditto Trend & Discourse Research — Agent Memory, 2026 Q1–Q2

*Compiled 2026-05-14. Prior research already covered openclaw, hermes-agent, openhuman, mempalace, gbrain and the established commercial cohort (Mem0, Zep, Letta, Hindsight, SuperMemory, Mastra OM, Cognee, MemMachine, ByteRover). This brief is **everything else** that's actually trending right now, what practitioners are saying, and what it implies for Ditto.*

---

## 1. Top 10 trending memory tools/repos we should know (2026 Q1–Q2)

Numbers are best-available at time of capture. Velocity flagged where verified.

| # | Project | What it is | Why it's hot |
|---|---------|------------|--------------|
| 1 | **GenericAgent** (`lsdefine/GenericAgent`) — ~11.3K stars | Self-evolving agent that "crystallizes each task into a skill" and grows a layered L0–L4 memory tree (Meta Rules → Insight Index → Global Facts → Task Skills/SOPs → Session Archive) from a 3.3K-line seed. Runs in <30K context where peers use 200K–1M. | arXiv tech report (2026-04-21) + a viral "bootstrapped its own repo without me opening a terminal" claim. Sits at the intersection of memory + skill-tree learning that the field is converging on. |
| 2 | **OpenViking** (`volcengine/OpenViking`) — ~23.9K stars, ~228 stars/day | ByteDance's filesystem-paradigm context DB. L0 (≈100 tok abstracts) / L1 (≈2K tok overviews) / L2 (full data) tiered loading. Python+Rust+C++. v0.3.16 on 2026-05-09. | Big-lab credibility, multilingual docs, and a clear answer to "how do you reduce token spend without losing recall." |
| 3 | **SimpleMem / Omni-SimpleMem** (`aiming-lab/SimpleMem`) — ~3.2K stars | "Semantic lossless compression" for text + image + audio + video. Three stages: Semantic Structured Compression → Online Semantic Synthesis → Intent-Aware Retrieval Planning. Claims 43.24% F1 on LoCoMo @ ~550 tokens/query; Omni-SimpleMem 0.613 F1 on LoCoMo (+47% over prior best) and 0.810 on Mem-Gallery. | First credible **multimodal** memory framework. Released v0.2.0 April 2026. |
| 4 | **engram** (`Gentleman-Programming/engram`) — ~3.5K stars, 82 releases, latest 2026-05-14 | "One brain. Local or cloud. Agent-agnostic, single binary, zero dependencies." Single Go binary, SQLite + FTS5, MCP over stdio, TUI with Catppuccin theme. Works with Claude Code, OpenCode, Gemini CLI, Cursor, Windsurf, VSCode Copilot. | Hits the "I don't want Docker/Postgres/Redis for memory" pain dead-center. Extreme release cadence. |
| 5 | **Memvid** (`memvid/memvid`) | "Replace complex RAG pipelines with a serverless, single-file memory layer." All data, indices (full-text, vector, time), and metadata in one portable `.mv2` file modeled on video-encoding ideas (append-only "Smart Frames" with timestamps + checksums). Sub-5 ms retrieval. | Same anti-infrastructure mood as engram, but **one portable file** is the killer demo for solo devs and edge use. |
| 6 | **agentmemory** (`rohitg00/agentmemory`) — ~8.7K stars | Claims 95.2% R@5 on LongMemEval-S vs. 86.2% BM25 baseline, 92% token reduction (~170K tokens/year, ~$10/year). 51 MCP tools, 12 auto-capture hooks, 4-tier (working→episodic→semantic→procedural). Hybrid BM25 + vector + KG via Reciprocal Rank Fusion. v0.9.12 on 2026-05-13. | The benchmark numbers are the marketing. Auto-capture hooks ("zero manual config") resonate. |
| 7 | **CocoIndex** (`cocoindex-io/cocoindex`) — ~9.7K stars | "Incremental engine for long-horizon agents." Turns codebases, meeting notes, Slack, PDFs, video into live continuously-fresh context. When source changes, identifies affected records, propagates across joins/lookups, retires stale rows — minimal incremental processing. | Reframes memory as **data infra problem, not retrieval problem**. Hits 2026's obsession with "agents that run for weeks." |
| 8 | **MemSkill** (`ViktorAxelsen/MemSkill`) — ~475 stars | Stores **meta-memory skills** ("what to extract, how to remember, where to focus") that are learned, refined, reused across tasks. HuggingFace #3 Paper of the Day Feb 2026 (arXiv:2602.02474). | Academic-flavored but the meta-skill framing matches where commercial systems (Anthropic Dreaming, GenericAgent) are independently converging. |
| 9 | **AMA-Bench** (`AMA-Bench/AMA-Bench`) — ICLR 2026 Memory Agent workshop | "Agent Memory with Any length." Two-part benchmark: (a) real agentic trajectories with expert QA, (b) synthetic trajectories scaling to arbitrary horizons. Ships an "AMA-Agent" baseline using causality graphs + tool-augmented retrieval that beats prior baselines by **11.16 points** (57.22% avg). | The first benchmark that explicitly stops being dialogue-centric and tests **machine-generated agent-environment streams**. This is the eval the field has been quietly asking for. |
| 10 | **dream-skill** (`grandamenium/dream-skill`) — ~59 stars but signal-rich | Reimplements Anthropic's unannounced "Auto Dream" feature as a Claude Code skill: 4-phase consolidation (Orient → Gather Signal → Consolidate → Prune & Index), Stop-hook auto-trigger, 24h cadence. | Tiny repo, big signal: practitioners are already cloning Anthropic's memory features into the open before Anthropic ships them. |

**Honorable mentions / second tier:** RuVector (Rust GNN vector memory DB), ADK-Rust (zavora-ai's agent dev kit with memory), Pi Agent Rust, Cloudflare Agent Memory (private beta 2026 Agents Week — managed memory at the edge), Letta sleep-time agents (Letta 0.7.0+), CtxVault (multi-vault separation, FastAPI, Feb 2026 HN), Hippo (biologically-inspired, R-STDP decay, Apr 2026 HN), Agent Recall (SQLite + scoped entities + bitemporal history, Feb 2026 HN), Mengram (semantic+episodic+procedural triad, Feb 2026 HN), Elfmem ("blocks that can be calibrated…dreaming…peer-to-peer"). MemPalace itself: still at 52.2K stars but **the benchmark scandal (see §4) means it's now the cautionary tale, not the reference design**.

**Rust shift:** Per OSS Insight, 2023–24 Rust AI tools averaged 25 stars/day; 2026's wave averages **404 stars/day — 16×**. Stated rationale: "when an agent has root access and runs autonomously, memory safety isn't optional." Ditto should at least track this — if memory is a daemon, Rust ergonomics will matter.

---

## 2. Top 10 practitioner pains (quoted, sourced)

1. **"Every new session burns time rediscovering the repo."** *r/AI_Agents, r/ClaudeAI, early May 2026.* Cold boots and re-reading dominate the daily-tax complaints. The "switching between Claude Code, Codex, and Cursor resets context all over again" framing got 238+ upvotes. ([dev.to/liv_melendez_4be3c47ea998](https://dev.to/liv_melendez_4be3c47ea998/what-the-ai-agent-crowd-on-reddit-is-arguing-about-in-early-may-2026-4j7e))

2. **"Why does the response get slower and slower over time?"** Production users on Letta forums describe context "spiking right when context gets rich" — token cost and latency both balloon with trace length. ([letta.com/blog/sleep-time-compute](https://www.letta.com/blog/sleep-time-compute))

3. **Stale memories that become "confidently wrong rather than just outdated."** Multiple practitioner guides flag this as the **#1 unsolved problem**: a highly-retrieved memory about a user's employer is highly relevant *until it isn't*. ([blog.bymar.co](https://blog.bymar.co/posts/agent-memory-systems-2026/), [mem0.ai/blog/state-of-ai-agent-memory-2026](https://mem0.ai/blog/state-of-ai-agent-memory-2026))

4. **"When retrieval goes bad, the system gives you almost no useful explanation."** Cited by Marco at byMAR.CO as the single biggest production failure mode. Practitioners want **retrieval diagnostics**, not better retrieval. ([blog.bymar.co](https://blog.bymar.co/posts/agent-memory-systems-2026/))

5. **The 200-line CLAUDE.md cap is a joke.** showjihyun: *"no selective retrieval. A 200-line cap on a monorepo with years of decisions means most knowledge is discarded."* The file-as-memory pattern doesn't scale past trivial projects. ([dev.to/jihyunsama](https://dev.to/jihyunsama/memory-is-the-unsolved-problem-of-ai-agents-heres-why-everyones-getting-it-wrong-4066))

6. **Letta burns inference tokens on memory bookkeeping.** *"Every memory operation burns inference tokens. The agent spends a significant portion of its token budget on memory management rather than on the actual task."* — same author. ([dev.to/jihyunsama](https://dev.to/jihyunsama/memory-is-the-unsolved-problem-of-ai-agents-heres-why-everyones-getting-it-wrong-4066))

7. **Mem0's split-store drift.** A March 2026 benchmark write-up: Mem0's vector store and optional Neo4j graph "share no IDs and run independently…can drift out of sync — this is the single most important difference." Graph knows an entity is important; its facts live in the vector store under unrelated IDs. ([dev.to/juandastic](https://dev.to/juandastic/i-benchmarked-graphiti-vs-mem0-the-hidden-cost-of-context-blindness-in-ai-memory-4le3))

8. **Mem0 retrieval accuracy is mediocre; Zep is accurate but huge.** showjihyun: *"Mem0 achieves only 49% retrieval accuracy; Zep reaches 63.8% but requires 340× more memory per conversation."* ([dev.to/jihyunsama](https://dev.to/jihyunsama/memory-is-the-unsolved-problem-of-ai-agents-heres-why-everyones-getting-it-wrong-4066))

9. **MCP tool bloat eats context.** r/LocalLLaMA: *"the more tools/MCPs you have the more context it takes, which can make agents less reliable."* Memory MCPs ironically make agents dumber by crowding out everything else. ([popularai.org](https://www.popularai.org/p/why-ollama-and-llama-cpp-crawl-when-models-spill-into-ram-and-how-to-fix-it))

10. **"Context rot" is sudden, not linear.** Recent benchmarks across 18 LLMs (GPT-4.1, Claude 4, Gemini 2.5, Qwen 3) show models maintain 95% accuracy then "suddenly plummet to 60%" at unpredictable context lengths. Token cost scales linearly with steps; reliability falls off a cliff. ([mindstudio.ai/blog/what-is-context-rot-ai-agents](https://www.mindstudio.ai/blog/what-is-context-rot-ai-agents))

**Honorable mention pains:** EU AI Act high-risk compliance deadline (August 2026) requires automatically generated logs + post-market monitoring — most memory systems offer **no audit trail at all**. ([atlan.com/know/ai-agent-memory-governance](https://atlan.com/know/ai-agent-memory-governance/))

---

## 3. Top 10 practitioner asks (what people say they want)

1. **A *memory controller*, not a storage backend.** The repeated complaint in every state-of-2026 post: "Most memory discussions focus on storage backends, retrieval algorithms, and context injection, but the component that is consistently missing is the memory controller." ([blog.bymar.co](https://blog.bymar.co/posts/agent-memory-systems-2026/))

2. **Cross-tool memory portability.** Users switching between Claude Code / Codex / Cursor want one memory that follows them. agentmemory and engram are both winning here precisely because they're MCP-clean across all of them. ([github.com/rohitg00/agentmemory](https://github.com/rohitg00/agentmemory))

3. **Explainable retrieval.** "Tell me *why* this memory came back, and *why* this other one didn't." Effectively no commercial system delivers this today.

4. **Active forgetting with promotion/demotion rules.** showjihyun's four-component proposal: tiered personal memory with explicit rules, structured shared-state protocol (not shared files), active forgetting weighted by access frequency + age + cross-agent reinforcement, and **conflict-as-data**. ([dev.to/jihyunsama](https://dev.to/jihyunsama/memory-is-the-unsolved-problem-of-ai-agents-heres-why-everyones-getting-it-wrong-4066))

5. **Bitemporal history (kept, not deleted).** Agent Recall's Nardit: "bitemporal history that archives rather than deletes outdated facts" + Mem0's state-of-2026 piece: "A user whose profile shows a move from New York to San Francisco should have both facts retained with the transition understood." ([HN 47165499](https://news.ycombinator.com/item?id=47165499))

6. **Sleep-time / background consolidation as a first-class primitive.** Letta sleep-time agents, Anthropic Auto-Dream, dream-skill (community port), GenericAgent's session-archive cron — convergent. Practitioners now expect background memory work to be **standard, not exotic**. ([anthropic dreaming](https://www.mindstudio.ai/blog/claude-dreaming-feature-self-improving-agent-memory))

7. **Audit trails for compliance.** Anthropic shipped this: "All memory changes are logged, with audit trails for each session and agent…ability to roll back, redact." Becoming table stakes for enterprise. ([computerworld.com](https://www.computerworld.com/article/4056366/anthropic-adds-memory-to-claude-for-team-and-enterprise-plan-users.html))

8. **Single-binary / zero-infra deployment.** engram, Memvid, ByteRover all converging on "no Postgres, no Redis, no vector DB service." Solo devs and self-hosters reward this hard.

9. **Multi-signal retrieval, not just cosine similarity.** Marco's six-thing list puts this at #3. agentmemory's RRF over BM25+vector+KG is the practical implementation.

10. **Workload-specific memory shapes.** Marco's takeaway: *"there probably will not be one memory winner."* Coding agents, support agents, research agents, copilots have different memory shapes. Practitioners want **opinionated presets** per workload, not a giant abstract toolkit. ([blog.bymar.co](https://blog.bymar.co/posts/agent-memory-systems-2026/))

---

## 4. Surprise findings (things that should change Ditto's strategy)

**A. The MemPalace scandal is the defining methodology event of the year.** Headline 96.6% LongMemEval R@5 was *a ChromaDB score*. Issue #214 by @hugooconnor reproduced 93.8% with a fresh **Rust BM25** implementation. Modes that actually use MemPalace's "palace" architecture score **lower** (Rooms 89.4%, AAAK 84.2%). The team quietly corrected, but the community now defaults to skepticism on every memory benchmark headline. ([github.com/MemPalace/mempalace/issues/214](https://github.com/MemPalace/mempalace/issues/214), [explainx.ai](https://explainx.ai/blog/mempalace-local-ai-memory-github)) → **Implication for Ditto:** if Ditto publishes benchmarks, *show the matched-conditions BM25 baseline on the same haystack and same retrieval mode*. Anything less will be torn down within hours.

**B. Embeddings are getting de-emphasized.** Supermemory's 99% SOTA flow *ditched vector embeddings entirely* in favor of agentic search ("the single biggest unlock — eliminating the semantic-similarity trap"). ByteRover stores everything as **human-readable markdown** with zero vector DB and beats Hindsight by 14.3 points on multi-hop. Agent Recall deliberately uses **keyword + scope filtering**. This is a real shift — Ditto should not assume embeddings/vector DBs are required. ([supermemory blog](https://blog.supermemory.ai/we-broke-the-frontier-in-agent-memory-introducing-99-sota-memory-system/), [byterover.dev](https://www.byterover.dev/blog/benchmark-ai-agent-memory))

**C. swyx flagged memory as the bottleneck on Latent Space (2026-04-23 crossover with Jacob Effron).** *"Context length is the slowest scaling factor in LLMs… Memory is probably gonna be the biggest limiting constraint."* And: *"Whatever memory or personalization system we end up with will probably determine what you end up choosing much more than what is currently the case."* Validates Ditto's thesis at the highest practitioner level. ([latent.space/p/unsupervised-learning-2026](https://www.latent.space/p/unsupervised-learning-2026))

**D. Karpathy's "agent wiki" pattern is going mainstream.** Sequoia Ascent 2026 (Karpathy): conversations → daily logs → wiki → injected back into next session. Anthropic's Auto-Dream + agentmemory's 4-tier consolidation + GenericAgent's L0–L4 + ByteRover's Context Tree are all variations on this. The convergence is not coincidental — it's the **emerging standard shape**. ([karpathy.bearblog.dev/sequoia-ascent-2026](https://karpathy.bearblog.dev/sequoia-ascent-2026/))

**E. Anthropic shipped enterprise-grade memory in April 2026 with case studies that are *very* strong.** Memory in Claude Managed Agents (2026-04-23): filesystem-stored, exportable, scoped, audited. **Harvey: ~6× task completion. Rakuten: 27% cost cut, 34% latency cut, 97% error reduction. Wisedocs: 50% review-time cut with Dreaming on.** Anything Ditto pitches to enterprise needs to clear that bar on **audit + scoping + portability** even before it claims accuracy advantages. ([computerworld.com](https://www.computerworld.com/article/4056366/anthropic-adds-memory-to-claude-for-team-and-enterprise-plan-users.html), [edtechinnovationhub.com](https://www.edtechinnovationhub.com/news/anthropic-brings-persistent-memory-to-claude-managed-agents-in-public-beta))

**F. ByteDance (volcengine) is shipping serious open memory infra.** OpenViking's 228 stars/day with multilingual EN/中/日 docs is a real geographic signal. Combined with Qwen overtaking Llama in cumulative downloads and Z.ai/MiniMax/Tencent shipping agent-style models, the **Asian open-source agent stack is now a peer competitor**, not a footnote. ([ossinsight.io](https://ossinsight.io/blog/agent-memory-race-2026), [technologyreview.com](https://www.technologyreview.com/2026/02/12/1132811/whats-next-for-chinese-open-source-ai/))

**G. Voice agents are the unexpected high-growth memory consumer.** Mem0's State of 2026 calls out voice (ElevenLabs, LiveKit) as the fastest-growing integration category with a *qualitatively different memory problem* from text agents (latency budgets, intent extraction from incomplete utterances). Nobody is serving this well yet. ([mem0.ai/blog/state-of-ai-agent-memory-2026](https://mem0.ai/blog/state-of-ai-agent-memory-2026))

**H. The "single portable file" memory pattern is winning over "managed service" with solo devs.** engram (Go binary), Memvid (.mv2 file), ByteRover (markdown files) all hit the same hot button. Mem0 SaaS, Zep Cloud, Letta Cloud all get the "too much infra, credit-based pricing, steep learning curve" complaint. ([evermind.ai](https://evermind.ai/blogs/zep-alternative))

**I. Self-evolving / skill-tree memory is its own emerging category.** GenericAgent (lsdefine), MemSkill (academic, HF #3 paper), EvoSkill (sentient-agi) all argue memory **is** skill. This is upstream of Ditto's current framing — if Ditto positions purely as a retrieval system, it risks looking dated next to skill-tree narratives. ([github.com/lsdefine/GenericAgent](https://github.com/lsdefine/GenericAgent), [github.com/sentient-agi/EvoSkill](https://github.com/sentient-agi/EvoSkill))

**J. ICLR 2026 had a dedicated Memory Agent workshop.** MemoryAgentBench (`HUST-AI-HYZ/MemoryAgentBench`) defined four core competencies: *accurate retrieval, test-time learning, long-range understanding, selective forgetting*. AMA-Bench landed in the same venue. Memory is now an **academic sub-field with its own ICLR workshop** — Ditto should map onto these four axes explicitly.

---

## 5. What Ditto should incorporate from this signal (concrete)

1. **Be Pareto-honest about benchmarks.** Always publish: (a) headline number, (b) matched-conditions BM25 baseline on the **same haystack at full corpus scale, not 50-session subsets**, (c) a clearly labeled "what part of the pipeline contributes how much" ablation. Pre-empt the MemPalace pattern. (The Issue #214 thread is the playbook for how skeptics will read your numbers.)

2. **Lead with the memory controller, not the store.** The single most repeated practitioner complaint in 2026 H1 is that controllers (the *what to write, when to update, what to expire*) are missing. Position Ditto's controller as the headline feature; treat backend as pluggable.

3. **Ship retrieval explainability as a first-class API.** `recall(query) → {result, why_retrieved, rejected_candidates, scoring_breakdown, confidence}`. No one has it. Marco called it "one of the biggest failures." Easy differentiator.

4. **Bitemporal facts by default.** Adopt Agent Recall's pattern: archive, don't delete. Mem0's piece explicitly says "should have both facts retained with the transition understood." This is also EU AI Act-friendly.

5. **Native consolidation cycle.** Background sleep-time compute is now the convergent standard (Letta, Anthropic, Anthropic-via-dream-skill, GenericAgent). Ditto should ship a documented `consolidate()` cycle with 4 phases (orient → gather → consolidate → prune) modeled on what Anthropic standardized, but **operate over Ditto's bitemporal store, not Anthropic's flat-file memory**.

6. **Zero-infra deploy mode + one-MCP install.** Match engram's "single binary, no Docker/Redis/Postgres" baseline as the default for solo devs and the install-on-laptop demo. Cross-tool MCP cleanliness (Claude Code, Cursor, Codex, Gemini CLI, Windsurf, VSCode Copilot, OpenCode) is now table stakes.

7. **Workload presets, not toolkit.** Ship `ditto.preset.coding`, `ditto.preset.support`, `ditto.preset.research` with different defaults (tiering, forgetting curves, signal weights). Marco's takeaway is becoming the consensus: no single memory shape wins.

8. **Audit trail + scoping out of the box.** Per Anthropic's enterprise feature: every write logged, every memory scoped, rollback/redact available. Without this, Ditto can't pitch into anyone enforcing EU AI Act in August 2026.

9. **Map explicitly onto MemoryAgentBench's four competencies** in marketing and eval: accurate retrieval, test-time learning, long-range understanding, selective forgetting. This is the academic vocabulary practitioners are starting to use; speaking it = legitimacy.

10. **Conflict-as-data.** Don't pick a winner between contradicting facts; store both with provenance. This is showjihyun's #4 ask, Marco's #2 ("preserve update semantics"), and Mem0's NYC→SF example. It's also a precondition for trustworthy audit trails.

11. **Don't bet the architecture on embeddings.** Both Supermemory (99% SOTA experimental) and ByteRover (92.2% LoCoMo) ditched vector search. Keep embeddings optional/pluggable; don't make them load-bearing.

12. **Take voice agents seriously.** Mem0 flagged it as the fastest-growing category. If Ditto's API supports streaming/partial-utterance writes and sub-100ms retrieval out of the box, that's a defensible wedge nobody else is filling.

---

## 6. Citations

### Trending repos & analyses
- OSS Insight, "The Agent Memory Race of 2026" — https://ossinsight.io/blog/agent-memory-race-2026
- OSS Insight, "The Rust Shift" — https://ossinsight.io/blog/rust-ai-agent-infrastructure-2026
- GenericAgent — https://github.com/lsdefine/GenericAgent (arXiv tech report 2026-04-21)
- OpenViking (ByteDance/volcengine) — https://github.com/volcengine/OpenViking
- SimpleMem / Omni-SimpleMem — https://github.com/aiming-lab/SimpleMem (v0.2.0, April 2026)
- engram — https://github.com/Gentleman-Programming/engram (v1.15.12, 2026-05-14)
- Memvid — https://github.com/memvid/memvid + https://memvid.com/
- agentmemory — https://github.com/rohitg00/agentmemory (v0.9.12, 2026-05-13)
- CocoIndex — https://github.com/cocoindex-io/cocoindex
- MemSkill — https://github.com/ViktorAxelsen/MemSkill (arXiv:2602.02474, HF #3 Feb 2026)
- AMA-Bench — https://github.com/AMA-Bench/AMA-Bench (ICLR 2026 Memory Agent workshop, arXiv:2602.22769)
- MemoryAgentBench — https://github.com/HUST-AI-HYZ/MemoryAgentBench (ICLR 2026)
- dream-skill — https://github.com/grandamenium/dream-skill
- ByteRover — https://www.byterover.dev/blog/benchmark-ai-agent-memory (arXiv:2604.01599)
- Awesome-AI-Memory (Chinese curated KB) — https://github.com/IAAR-Shanghai/Awesome-AI-Memory
- Agent Memory Paper List — https://github.com/Shichun-Liu/Agent-Memory-Paper-List

### MemPalace controversy
- Issue #214 (hugooconnor) — https://github.com/MemPalace/mempalace/issues/214
- Issue #29 (methodology) — https://github.com/milla-jovovich/mempalace/issues/29
- Danilchenko review — https://www.danilchenko.dev/posts/2026-04-10-mempalace-review-ai-memory-system-milla-jovovich/
- Nicholas Rhodes review — https://nicholasrhodes.substack.com/p/mempalace-ai-memory-review-benchmarks
- Vectorize debunk — https://vectorize.io/articles/mempalace-benchmarks
- Cybernews recap — https://cybernews.com/ai-news/milla-jovovich-mempalace-memory-tool/
- explainx.ai (Reddit sentiment) — https://explainx.ai/blog/mempalace-local-ai-memory-github

### HN show-HN threads (Q1–Q2 2026)
- CtxVault (Filippo Venturini, Feb 2026) — https://news.ycombinator.com/item?id=47136585
- Agent Recall (Max Nardit, Feb 2026) — https://news.ycombinator.com/item?id=47165499
- Mengram (Feb 2026) — https://news.ycombinator.com/item?id=47151177
- Hippo (Apr 2026) — https://news.ycombinator.com/item?id=47667672
- Elfmem (May 2026) — https://news.ycombinator.com/item?id=47980686
- "Why agent memory needs more than RAG" — https://news.ycombinator.com/item?id=47060572

### Practitioner commentary
- Marco / byMAR.CO, "Agent Memory Systems in 2026: What Actually Matters" — https://blog.bymar.co/posts/agent-memory-systems-2026/
- showjihyun, "Memory Is the Unsolved Problem of AI Agents" — https://dev.to/jihyunsama/memory-is-the-unsolved-problem-of-ai-agents-heres-why-everyones-getting-it-wrong-4066
- juandastic, "Graphiti vs Mem0: Context Blindness" — https://dev.to/juandastic/i-benchmarked-graphiti-vs-mem0-the-hidden-cost-of-context-blindness-in-ai-memory-4le3
- Reddit weekly aggregator (May 2026) — https://dev.to/liv_melendez_4be3c47ea998/what-the-ai-agent-crowd-on-reddit-is-arguing-about-in-early-may-2026-4j7e
- The New Stack, "Why your AI agent doesn't actually remember anything" (Ed Huang, 2026-05-11) — https://thenewstack.io/agent-memory-decay-contamination/
- MindStudio, "What Is Context Rot in AI Agents" — https://www.mindstudio.ai/blog/what-is-context-rot-ai-agents

### Vendor positions (current as of 2026)
- Mem0, State of AI Agent Memory 2026 — https://mem0.ai/blog/state-of-ai-agent-memory-2026
- Mem0, "Context Window Behaves Like RAM, Not Storage" — https://mem0.ai/blog/context-window-is-ram-not-storage-why-most-agent-failures-happen-how-to-fix-them-in-2026
- Letta, Sleep-time Compute — https://www.letta.com/blog/sleep-time-compute
- Supermemory, 99% SOTA experimental — https://blog.supermemory.ai/we-broke-the-frontier-in-agent-memory-introducing-99-sota-memory-system/

### Anthropic memory (April 2026)
- Computerworld — https://www.computerworld.com/article/4056366/anthropic-adds-memory-to-claude-for-team-and-enterprise-plan-users.html
- EdTech Innovation Hub — https://www.edtechinnovationhub.com/news/anthropic-brings-persistent-memory-to-claude-managed-agents-in-public-beta
- TestingCatalog — https://www.testingcatalog.com/anthropic-launches-memory-in-claude-agents-for-enterprise/
- Claude API docs (Dreams) — https://platform.claude.com/docs/en/managed-agents/dreams
- VentureBeat (Dreaming) — https://venturebeat.com/technology/anthropic-introduces-dreaming-a-system-that-lets-ai-agents-learn-from-their-own-mistakes
- MindStudio Dreaming explainer — https://www.mindstudio.ai/blog/claude-dreaming-feature-self-improving-agent-memory

### Influencer / thought-leader signal
- Latent Space crossover with Jacob Effron (2026-04-23) — https://www.latent.space/p/unsupervised-learning-2026
- Karpathy, Sequoia Ascent 2026 — https://karpathy.bearblog.dev/sequoia-ascent-2026/
- Taranjeet (Mem0) on memory pipeline — https://x.com/taranjeetio/status/1920139644861378712
- Dhravya Shah (Supermemory) 99% SOTA — https://x.com/DhravyaShah/status/2035517012647272689

### Governance / regulatory
- Atlan, AI Agent Memory Governance — https://atlan.com/know/ai-agent-memory-governance/
- TianPan, Decision Provenance in Agentic Systems (2026-04-19) — https://tianpan.co/blog/2026-04-19-decision-provenance-agentic-systems
- SVRN, AI Agent Provenance — https://svrn.net/news/provenance-for-agent-actions

### YC W26 & Chinese open source
- TechCrunch, "16 of the most interesting startups from YC W26 Demo Day" — https://techcrunch.com/2026/03/26/16-of-the-most-interesting-startups-from-yc-w26-demo-day/
- BuildMVPFast, YC W26 Agent Infrastructure Boom — https://www.buildmvpfast.com/blog/yc-w26-batch-agent-infrastructure-boom
- MIT Technology Review, "What's next for Chinese open-source AI" (2026-02-12) — https://www.technologyreview.com/2026/02/12/1132811/whats-next-for-chinese-open-source-ai/

---

*Anything older than 6 months from 2026-05-14 has been flagged. Most cited items are dated within Q1–Q2 2026; the Letta sleep-time compute paper (arXiv:2504.13171, April 2025) is the oldest load-bearing reference and is **stale on dates but the technique it introduced is now mainstream** — that staleness is itself a signal.*
