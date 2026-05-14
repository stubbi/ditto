# Memory Systems for Agent Harnesses: State of the Art and the Ditto Position

*Research dive for github.com/stubbi/ditto. Last updated 2026-05-14.*

This report synthesizes the current research and product landscape on agent memory (2024 H2 through 2026 Q2) and proposes a concrete architecture for **Ditto** that is intended to be **Pareto-better** than the memory layers shipped by hermes-agent, openclaw, openhuman, mempalace, and gbrain. The goal is to make architectural commitments that hold up for the next 12–24 months under real workloads — not to enumerate every academic system.

Industry-standard caveat: many cited benchmark numbers below are vendor-reported and not independently audited. Treat absolute scores as illustrative; treat *gaps between systems* as more informative than absolute deltas.

---

## 1. The taxonomy of agent memory

A useful agent memory system varies along ~12 orthogonal axes. Most current systems lock in 3–4 of them and leave the rest implicit (and broken).

| Axis | Range | Notes |
|---|---|---|
| **Scope** | turn / session / agent / user / tenant / org | Cross-thread retrieval is where naive systems fail (#1505 openhuman cross-chat). |
| **Durability** | ephemeral / session-scoped / durable / archival | Some "memory" is just a context buffer (LangGraph short-term); some is a forever store. |
| **Structure** | flat text / typed records / KG / hierarchical tree / hybrid | hermes ships flat MD; Zep/Graphiti ships typed temporal KG; openhuman ships a hierarchical Markdown tree. |
| **Write-time computation** | passive log / LLM-extract / KG-extract / regex-extract / embed-only | Mem0 does single-pass LLM extract; gbrain does zero-LLM regex KG; A-MEM does LLM contextual notes. |
| **Read-time strategy** | semantic only / BM25 only / hybrid+RRF / hybrid+rerank / KG-traversal+hybrid+rerank | Hindsight's 4-strategy fusion is current SOTA in the wild. |
| **Consolidation cadence** | none / immediate / batch (daily) / dream-cycle / on-eviction | Mem0 is online; ADM/SCM do explicit sleep cycles; openclaw's "dreaming" is contested (#65963, #67413). |
| **Eviction policy** | unbounded / TTL / LRU / importance×recency / Ebbinghaus decay | MemoryBank introduced Ebbinghaus decay; almost nobody actually uses it in production. |
| **Provenance** | none / metadata / signed receipts / Merkle / content-addressed | Signed receipts (Ed25519+JCS) are an emerging standard — see Microsoft agent-governance-toolkit #1499. |
| **Multi-writer semantics** | single writer / gateway / app-managed locks / CRDT | MemPalace #1497 *is* the smoking gun — substrate isn't process-safe under concurrent writers. |
| **Multi-tenant isolation** | none / namespace / RLS / database-per-tenant / federated | pgvector+RLS is the pragmatic default in 2026. |
| **Eval methodology** | none / retrieval recall / end-to-end QA / behavioral tasks (Terminal-Bench) | Retrieval recall ≠ correctness; BEAM specifically punishes this distinction. |
| **Distribution model** | in-process / sidecar / managed SaaS / MCP server | MCP is becoming the lingua franca; in-process still wins on p50. |

The honest summary: **no single system dominates more than 5–6 of these axes simultaneously**. That is Ditto's opening.

---

## 2. State of the art today (2025–2026)

### 2.1 Typed knowledge graphs with bi-temporal validity

The leading architectural pattern in 2026 for facts that change over time is the bi-temporal KG, pioneered in production by **Graphiti / Zep** ([arxiv 2501.13956](https://arxiv.org/abs/2501.13956)). Every edge carries four timestamps:

- `t_created`, `t_expired` — transaction time (when the system learned/forgot it)
- `t_valid`, `t_invalid` — valid time (when the fact actually held)

A new edge that semantically overlaps an existing edge triggers an LLM contradiction check; if the new edge contradicts the old, the old edge's `t_invalid` is set to the new edge's `t_valid` — invalidated, never deleted. This is what lets Zep beat Mem0 by 14.8 points on LongMemEval (63.8 vs 49.0 on GPT-4o, vendor-reported) specifically on knowledge-update and temporal-reasoning slices.

**Open KG variants worth knowing:**
- A-MEM (NeurIPS 2025) — Zettelkasten-style atomic notes with LLM-generated contextual links, evolving structure. Strong on multi-hop reasoning in long convos.
- LinkedIn's Cognitive Memory Agent — production-scale episodic+semantic+procedural layering.
- mempalace's wings/rooms/closets/drawers KG — neat metaphor, but the durability story is broken (#1078, #1091, #1329, #1497).

### 2.2 Hybrid retrieval done right

The 2026 production stack is converging:

1. **First stage**: dense (HNSW/DiskANN) + sparse (BM25 or SPLADE/BM42) recall, fused via **Reciprocal Rank Fusion** (RRF) — no scale normalization, robust to mixed score distributions.
2. **Second stage**: cross-encoder or **ColBERT late-interaction** rerank over top-50/100. ColBERT achieves p50 ~20ms vs cross-encoder ~45ms.
3. **Third stage (optional)**: KG traversal seeded by the reranked candidates (Graphiti's approach).

Mem0's April 2026 token-efficient algorithm fuses semantic + BM25 + entity match into one score, claiming 91.6 LoCoMo / 93.4 LongMemEval / 64.1 BEAM-1M / 48.6 BEAM-10M at <7k tokens per call. The compounding insight from BEAM is that **retrieval recall is not a sufficient metric** — BEAM tests contradiction resolution, abstention, knowledge updates, and temporal reasoning where recall-only systems can hit 90% recall and still fail the QA outright.

### 2.3 Episodic → semantic consolidation

This is the second-largest delta between toy systems and production systems. Three patterns:

- **Online consolidation** (Mem0): extraction at write time, immediate dedup against KG. Pro: never goes stale. Con: every write is on the latency-critical path; misses cross-session structure.
- **Sleep cycle / dream cycle** (SCM, ADM, generative-agents reflection): batch consolidation pass that promotes episodic events to semantic facts, generates higher-order reflections, applies counterfactual verification. Pro: catches structure. Con: opaque batch jobs, hard to debug, openclaw's #67413 shows the failure mode.
- **Reflection-on-read** (Mastra Observational Memory): two background agents — Observer compresses unobserved history into traffic-light-tagged bulleted observations, Reflector compresses the observation log when it grows. Stable, prompt-cacheable context window. Claims 94.87% on LongMemEval (gpt-5-mini), reportedly SOTA.

The empirical case for OM-style "two-agent watcher + dense text log" is the strongest *practical* one right now: it's simple, prompt-cacheable, and beats more elaborate KG systems on LongMemEval. The catch: it doesn't naturally support cross-session structured queries ("what did the user say about postgres last quarter?") — pure dense observation logs degrade under multi-tenant, multi-source workloads.

### 2.4 Mem0-style 4-tier memory (working/episodic/semantic/procedural)

This taxonomy — coming originally from cognitive science via ACT-R and Park's generative agents — is now standard:

- **Working memory**: in-context, current turn / agent scratchpad.
- **Episodic**: timestamped events, conversation turns, observations. Append-only.
- **Semantic**: distilled facts, user preferences, world knowledge. Mutable, with provenance back to episodes.
- **Procedural**: skills, workflows, code snippets, tool-call patterns. Voyager-style skill libraries, now mainstreamed by Anthropic Agent Skills (Oct 2025) — 62k+ stars on `anthropics/skills` within four months.

The dirty secret is that most "4-tier" implementations conflate semantic and episodic into a single embedding store and call it a day. The cleanly separated implementations (Letta/MemGPT, LinkedIn CMA, Hindsight's four networks) outperform on multi-hop tasks because procedural and semantic have different lifecycle and retrieval needs.

### 2.5 MemGPT / Letta hierarchical paging

MemGPT (Oct 2023, rebranded to Letta Sep 2024) is the "LLM as OS" thesis: virtual context with paging between physical (in-context) and disk (out-of-context) memory, where the agent itself issues page-in/page-out tool calls. Letta Code (Dec 2025) holds #1 on Terminal-Bench, vindicating the architecture for *long-horizon* agentic work even when token windows are large. The key insight Letta gets right: **the agent must be in the loop on what it remembers**. Pure passive logging loses the ability to deliberately remember things ("this matters, save it").

### 2.6 Generative Agents reflection chains

Park et al.'s memory stream + reflection ([arxiv 2304.03442](https://arxiv.org/abs/2304.03442)) is the canonical retrieval-scoring function: `score = α_recency · recency + α_importance · importance + α_relevance · relevance`. Recency is exponential decay; importance is LLM-rated 1–10; relevance is cosine. Almost every serious system since has some variant of this scoring, often with learned weights.

### 2.7 Signed / attested memory

This is the emerging frontier and Ditto's biggest opportunity. ContextSubstrate captures each agent run as a SHA-256 content-addressed pack. Cloudflare Agent Memory uses content-addressed message IDs for idempotent re-ingestion. The Microsoft agent-governance-toolkit issue #1499 specifies Ed25519+JCS offline-verifiable decision receipts. Academic work ([arxiv 2506.13246](https://arxiv.org/abs/2506.13246)) proposes Merkle-chained immutable agent memory.

**The conclusion: a memory write should produce a content-addressed, signed receipt as a first-class part of the API.** None of hermes, openclaw, openhuman, mempalace, or gbrain do this today. Hermes issue #11692 is the admission.

---

## 3. Benchmark landscape

### 3.1 What each benchmark measures

| Benchmark | Year | What it tests | Why it matters |
|---|---|---|---|
| **LongMemEval** (ICLR 2025) | ~500 sessions, info extraction, multi-session reasoning, temporal, knowledge-update, abstention | The de-facto leaderboard. Mem0 93.4, Mastra OM 94.87 (gpt-5-mini), Hindsight 91.4 (Gemini-3 Pro), SuperMemory 81.6, ByteRover 92.8, Zep 63.8 (GPT-4o), MemPalace ~84 | Becoming saturated; useful as a floor. |
| **LoCoMo** | 32-session dialogues, ~600 turns, multimodal | Maharana et al.'s long-term convo benchmark; Mem0 91.6; MemMachine 0.8487 llm_score | Better at very-long dialogue stress. |
| **BEAM** (ICLR 2026) | 2,000 questions across 100 conversations, scaling to 10M tokens | Specifically tests 10 abilities including contradiction resolution that recall can't capture | The right benchmark for 2026; recall-only systems lose +155% to architected memory at 10M tokens. |
| **MemBench** (ACL 2025) | Effectiveness, efficiency, capacity, temporal efficiency | Adds capacity/efficiency dimensions | Underused but the metric set is right. |
| **DMR** (Deep Memory Retrieval) | Multi-session deep recall | Zep's preferred benchmark | Largely superseded by LongMemEval. |
| **Terminal-Bench** | Long-horizon agentic terminal tasks | Letta Code #1 in Dec 2025 | Behavioral, not memory-specific, but the harness with the best memory tends to win. |

### 3.2 The retrieval-recall vs end-to-end QA gap

This is the single most important methodological point. A system can retrieve 9/10 relevant facts and still answer wrong because:
- The 1 missing fact is the critical one (contradiction resolution).
- The retrieved facts are stale and the agent doesn't know it.
- Multi-hop reasoning fails despite per-hop recall.
- Abstention questions: the system retrieves *something* and confidently hallucinates when it should refuse.

BEAM explicitly tests these. Mem0's own data shows their token-efficient algorithm clears BEAM-1M at 64.1% and BEAM-10M at 48.6% — meaning roughly half of 10M-token-scale questions are still failing even with the best memory in the wild.

### 3.3 What benchmarks are missing (Ditto's positioning)

Three benchmarks that should exist but don't, where Ditto could anchor a credibility claim:

- **Provenance correctness**: given a claimed answer, can the system point to the exact memory write(s) that produced it, and are those writes attestable? No public benchmark today.
- **Multi-tenant isolation under adversarial queries**: given two tenants with overlapping topics, can tenant A retrieve none of tenant B's facts under prompt-injection attempts?
- **Crash-consistent memory**: kill -9 mid-write, restart, verify no torn writes and no lost-but-acknowledged writes. Database fundamentals applied to agent memory; almost no public system tests this.

These are all axes where Ditto can ship a benchmark *and* the SOTA score, simultaneously.

---

## 4. Where the incumbents fail

### 4.1 hermes-agent
- **Wins**: simple, in-tree, ubiquitous, FTS5 session search is fast.
- **Loses**: flat `MEMORY.md` + `USER.md` doesn't scale beyond ~100 facts. No quality gate on skill creation (issue #13265). Name-only dedup means semantic duplicates pile up. Mechanical timer-driven creation produces dead skills. Dead `related_skills` field signals an architectural overhang. PR #25302 *closing* in-tree memory plugins to new PRs is an admission: the model can't scale to community contributions.

### 4.2 openclaw
- **Wins**: takes the problem seriously; ships three parallel memory systems (memory-core, memory-lancedb, memory-wiki) so users can pick.
- **Loses**: three half-systems means none of them are battle-tested. Active issues: multi-slot memory (#60572), pre-reset/pre-compaction flushes (#45608, #81804), active-memory deadlock (#79026), indexing leaks (#71285), dreaming session sprawl (#65963, #67413), token-budget waste (#9157), server-side compaction (#10213). The architecture is contested *internally*.

### 4.3 openhuman
- **Wins**: Memory Tree is genuinely elegant — canonicalize every connected source into ≤3k token chunks, build a hierarchical summary tree, store in SQLite + Obsidian-compatible `.md`. Auto-fetches every 20min from Composio.
- **Loses**: cross-chat memory failures (#1505), UTF-8 char-boundary panics on `body_preview` slicing (#1595, #1654), no durable memory review queue (#1539). Correctness bugs in primitives are a tell that the system was built outside-in.

### 4.4 mempalace
- **Wins**: wings/rooms/closets/drawers KG + temporal validity, 96.6% LongMemEval R@5.
- **Loses**: HNSW corruption at scale — 141GB bloat (#1078), 582GB (#1091), 1.9TB (#1329). The single-writer / gateway question (#1497) is the smoking gun: **the substrate isn't process-safe**. Plus a credibility cloud (#27, 335 reactions about over-claiming). This is what happens when you ship a beautiful KG on top of a substrate that wasn't designed for concurrent agent traffic.

### 4.5 gbrain
- **Wins**: compiled-truth + timeline page format; zero-LLM regex KG extraction at write time (no extraction cost on the hot path); hybrid retrieval (pgvector HNSW + Postgres tsvector + RRF + multi-query + 4-layer dedup + compiled-truth boost); BrainBench-Real session capture; PGLite for local dev.
- **Loses**: schema-migration treadmill (30+ issues, every minor version breaks brains). Regex extraction is fast but brittle on unstructured tool outputs. No procedural memory.

### 4.6 The big commercial systems
- **Mem0**: best LongMemEval/LoCoMo numbers, token-efficient, broad ecosystem (CrewAI, Strands, Flowise). But: weak temporal reasoning (no bi-temporal model), entity-extraction artifacts at scale, vendor-lock-y graph backend.
- **Zep/Graphiti**: best temporal reasoning, Apache 2.0 Graphiti core. But: Zep Community Edition deprecated April 2025 — can't self-host the full stack. Neo4j dependency.
- **Letta**: best long-horizon agentic perf (Terminal-Bench #1 with Letta Code, Dec 2025). But: heavyweight; the agent-managed paging loop adds tool-call overhead.
- **SuperMemory**: clean API, broad surface (memory+RAG+profiles+connectors). But: closed source, no OSS, enterprise-only self-host.
- **Hindsight**: 4-network architecture (World/Experience/Opinion/Entity), 4-strategy parallel retrieval, #1 BEAM-10M. But: bespoke, smaller ecosystem.
- **Mastra OM**: simplest architecture that hits SOTA on LongMemEval. But: dense-text observation logs don't naturally do structured cross-tenant queries.
- **Cognee**: vector+graph, fully local. But: smaller velocity, less battle-tested at scale.

---

## 5. The Pareto frontier — axes Ditto must dominate

For each axis: current leader, floor (must clear), ceiling (should aim for), architectural commitment to clear ceiling.

| Axis | Leader | Floor | Ceiling | What it takes |
|---|---|---|---|---|
| **LongMemEval (frontier model)** | Mastra OM (94.87, gpt-5-mini) | 90+ | 95+ | Observational summarization + hybrid retrieval + rerank |
| **BEAM-10M** | Hindsight ~89% | 65 | 90+ | Multi-strategy retrieval, KG with contradiction resolution, abstention training |
| **Temporal correctness** | Graphiti/Zep | bi-temporal edges | bi-temporal + counterfactual verification | TKG with `t_valid/t_invalid` + reflection-pass invalidation |
| **Token cost per retrieval** | Mem0 (<7k) | <10k | <5k | Compressed observation logs + sparse KG dump, learned compressor |
| **p50 retrieval latency** | gbrain (Postgres-native), ColBERT-class | <100ms | <30ms | In-process Postgres+pgvector / DuckDB-VSS; ColBERT rerank, async prefetch |
| **Crash-consistency** | None publicly | WAL + fsync on commit | Linearizable single-writer with content-addressed receipts | Postgres or SQLite as substrate; never invent durability |
| **Multi-writer safety** | Postgres-backed systems | RLS + advisory locks | Single-writer harness boundary with append-only event log + downstream consolidator | Architectural choice: only the harness writes; agents emit events |
| **Provenance / attestation** | ContextSubstrate (content-addressed only) | content-addressed | Ed25519-signed receipts + Merkle log over consolidation cycles | First-class signing key per harness install; receipt API |
| **Multi-tenant isolation** | pgvector+RLS | per-tenant namespace + RLS | RLS + per-tenant collections + adversarial isolation test | RLS by default; isolation regression tests in CI |
| **Procedural memory** | Voyager / Anthropic Agent Skills | skill library | skill library + provenance + metabolism (GC of dead skills) | Skill records as first-class typed memory with lifecycle |
| **Eval methodology** | Mem0, Hindsight (ship their numbers) | BEAM-10M + LongMemEval in CI | BEAM + LongMemEval + Ditto-Provenance-Bench + crash-consistency suite in CI | In-tree fixtures, regression gates on every PR |
| **Distribution** | Mem0 (broad), Anthropic Memory MCP (standard) | in-process SDK + MCP server | in-process SDK + MCP server + sidecar daemon for shared multi-agent | One Rust core, three transports |

The unique combination Ditto can claim: **bi-temporal KG + observational summarization + signed receipts + crash-consistent Postgres substrate + skills as first-class memory** — none of the incumbents combine all five.

---

## 6. Ditto's proposed memory architecture

### 6.1 Typed memory model

Five typed slots, each with explicit lifecycle:

```
Working   — in-context, current turn. TTL = end-of-turn. Not durable.
Episodic  — append-only event log of agent observations/actions/tool calls.
            Content-addressed (SHA-256 of canonical JSON). Immutable.
Semantic  — distilled facts. Mutable via supersession (never delete).
            Bi-temporal: (t_created, t_expired, t_valid, t_invalid).
            Each version links back to the episodic events that supported it.
Procedural — skills, workflows, code snippets, tool patterns.
            Each skill has: prompt, code, tests, success metrics, deprecation marker.
            Skill metabolism: skills that haven't fired in N days OR fail their tests
            get auto-marked deprecated and excluded from retrieval.
Reflective — higher-order observations / patterns / user-model facts.
            Generated by consolidation, not by the user agent. Provenanced.
```

The point of distinguishing reflective from semantic is to keep the *raw* user-provided semantic record clean and auditable; reflections are derived and disclosable as such.

### 6.2 Write path: single-writer harness boundary

**The harness is the only writer.** Agents emit events; the harness commits them. This is the lesson from MemPalace #1497 and openclaw #79026. Concrete:

1. Agent emits an `Observation`, `ToolCall`, or `Reflection` event over an in-process channel (or MCP for sidecar mode).
2. Harness assigns `event_id = sha256(canonical_json(payload))` — content-addressed, idempotent re-ingest.
3. Harness writes to the episodic event log inside a Postgres transaction. WAL handles durability. No torn writes possible.
4. Harness signs the event with the install's Ed25519 key; receipt = `(event_id, prev_event_id, signature, timestamp)`. Receipts form a hash chain.
5. Asynchronous consolidator (separate worker, can be in-process or sidecar) reads the event log and produces semantic + reflective writes. Consolidator output is itself signed and content-addressed.

This gives Ditto: **linearizable writes, crash-consistency for free (Postgres WAL), idempotent re-ingestion, hash-chained audit log, no multi-writer races, no torn updates.** That's a Pareto sweep over every incumbent.

### 6.3 Read path: hybrid retrieval with cost-aware modes

Three modes, agent picks per query (defaulting to `standard`):

- **`cheap`**: BM25 only (Postgres tsvector) + KG entity match. p50 < 5ms. No LLM calls. Good for ID lookups, recent-fact recall.
- **`standard`**: BM25 + pgvector HNSW → RRF fusion → ColBERT-class rerank over top-50 → KG traversal expansion (1 hop). p50 < 50ms. No LLM calls on the hot path.
- **`deep`**: standard + query expansion (small LLM, async-prefetched) + cross-encoder rerank + multi-hop KG. p50 ~200ms. Used for hard multi-hop questions, escalated when standard returns low confidence.

Recency, importance, and relevance (Park et al. scoring) compose into the final ranker; weights are learned per-tenant via offline regression on consolidated query logs (not on the hot path).

### 6.4 Consolidation loop

Two cadences, both background:

- **Online** (per N events, default N=20): scan recent episodic events, run a single-LLM-call extractor (Mem0-style single-pass) to propose semantic facts. New facts hit the KG contradiction-resolution check (Graphiti-style temporal invalidation). All writes are receipt-signed.
- **Dream cycle** (per session-close + per 24h): consolidator agent (Observer + Reflector pattern from Mastra OM) re-reads recent episodic + semantic, generates higher-order reflections, runs counterfactual verification (ADM-style) before committing reflections. Skill metabolism runs here: skills with `last_used > 30d` and `tests_pass < threshold` are deprecated.

The single-LLM-call extractor is on the write path *for online consolidation only*; the dream cycle is fully off-path. This separation is what lets Ditto match Mem0's token cost while exceeding it on temporal correctness.

### 6.5 Eval loop

Ship in-tree, run on every PR, regression-gated:

- **LongMemEval-M** fixture, 4 model targets (gpt-4o, gpt-5-mini, opus-4.7, sonnet-4.7). Target floor: 90% on each.
- **BEAM-1M** subset (sampled to fit CI budget) + full BEAM-10M nightly. Target floor: 65% / 48%.
- **Ditto-Provenance-Bench** (we ship it): given a Q&A pair, the system must return the exact set of episodic event_ids that produced the answer. Recall ≥ 0.95.
- **Ditto-Isolation-Bench** (we ship it): two-tenant adversarial corpus with overlapping topics; tenant A queries with prompt-injection attempts to surface tenant B's facts. Leak rate must be 0.
- **Crash-consistency suite**: kill -9 mid-write under load; verify no acknowledged-but-lost writes, no torn writes, no broken hash chain. Block the PR if any fail.

This eval surface is itself defensible — once we publish Ditto-Provenance-Bench and Ditto-Isolation-Bench, others have to either run them (catching up) or argue they don't matter (losing on enterprise dimensions).

### 6.6 Storage substrate

**Decision: Postgres with pgvector + tsvector as default; SQLite + sqlite-vec for embedded mode.**

Reasoning:
- pgvector at < 50M vectors is operationally trivial, supports HNSW, transactional with the relational record, and works with RLS for tenant isolation.
- Beyond 50M vectors per tenant, switch the index implementation to DiskANN/Vamana (SQL Server 2025 ships this; pgvector is gaining it; Milvus has it) — same data model, swappable index.
- DuckDB-VSS is tempting for analytical local-dev mode but has HNSW persistence caveats; we won't ship it as the primary substrate but support it for `ditto eval` workloads.
- Tigris and other object-store-backed vector DBs are great for archival/cold tier but not the hot read path.
- Explicitly **avoid Neo4j-as-substrate** (Zep's choice): operationally heavier, harder to RLS, weaker WAL story than Postgres. Model the KG as Postgres tables with edge rows + temporal columns.

The schema-migration pain that gbrain hit (30+ issues, every minor version breaks brains) is avoidable with: (a) zero destructive migrations — additive-only schemas + view layer; (b) memory format versioning baked into content-addressed receipts (you can always replay).

### 6.7 Multi-tenant story

- **Default**: single Postgres database, RLS by `tenant_id` on every memory table. Application sets `app.tenant_id` per-request. The reason this works is that pgvector indexes honor RLS at query time.
- **Source isolation**: every episodic event has a `source_id` (e.g., per-Composio-connector, per-MCP-server). Retrieval defaults to all sources for the tenant but supports source-scoped queries (`memory.search(scope=["github", "gmail"])`).
- **Federated query**: for enterprise multi-cluster, Ditto supports a federation layer where queries fan out to per-region installs, results merge via RRF over signed result-sets. Each install signs its result with its key; the federator verifies signatures before merging.
- **Database-per-tenant escape hatch**: for regulated tenants (healthcare, defense), a config flag flips to one Postgres database per tenant. Same code path; the harness just routes connections.

### 6.8 Integration surface

Three transports, one core:

- **In-process SDK** (Rust core, bindings for Python/TS): zero serialization overhead, used by the Ditto harness itself.
- **MCP server**: standard MCP surface for use with Claude Code, Cursor, third-party harnesses. Becomes the lingua franca exposure.
- **HTTP API + sidecar daemon**: for multi-agent multi-process setups where the harness is *not* the only consumer; daemon mediates writes so single-writer invariant holds across processes.

All three transports go through the same write-path validation and produce the same signed receipts.

---

## 7. Defensibility analysis

The moat is *composed*, not single-feature. Each axis individually is replicable; the combination is much harder.

| Component | How hard to copy | Why |
|---|---|---|
| Bi-temporal TKG with contradiction resolution | Medium | Graphiti is open-source Apache 2.0; replicable. Just hard to do well. |
| Hybrid retrieval + ColBERT rerank | Easy | Well-trodden; commodity. |
| Observational consolidation | Easy | Mastra OM is published. |
| Skill metabolism & lifecycle | Medium | Few systems do it right; requires real telemetry. |
| **Signed, content-addressed receipts on the write path** | Hard | Requires architectural commitment from day one; can't bolt on. |
| **Single-writer harness boundary** | Hard | Same — it's an invariant, not a feature. Once a system allows concurrent writers (every incumbent), retrofitting linearizability requires breaking changes. |
| **Crash-consistency** | Hard | Requires choosing the right substrate (Postgres) and never inventing storage. Most agent-memory startups built bespoke stores; they can't shed that without a rewrite. |
| **Provenance + Isolation benchmarks** | Hard to copy *credibly* | The first system to publish credible benchmarks defines the discourse. |
| Multi-tenant RLS | Medium | Easy to copy if on Postgres, hard if on Neo4j/bespoke. Zep is on Neo4j. |

**Who can catch up fastest?**
- **Mem0** — has the algorithm and ecosystem; would need to add signed receipts and harden temporal model. ~6 months.
- **Letta** — has the agent-OS frame; would need to ship multi-tenant + provenance. ~9 months.
- **Hindsight** — closest on retrieval quality; lacks provenance and substrate story. ~9 months.
- **Hermes / Openclaw / Openhuman** — would need to admit the current model is wrong and replumb. PR #25302 closing in-tree plugins suggests Hermes has already accepted this; they're more likely to *adopt* Ditto-shaped systems than build one.

**What Ditto must keep doing to stay ahead:**
1. Ship the benchmarks before competitors do. First mover defines what "good memory" means.
2. Keep the substrate story boring (Postgres). Resist the urge to invent storage. Every storage-invention story (mempalace HNSW bloat, gbrain schema treadmill) ends badly.
3. Build the procedural memory + skill metabolism story aggressively. Anthropic Agent Skills is the wedge.
4. Don't grow the data model. Five typed slots is the contract; new use-cases must fit, not extend.

---

## 8. Risks and unknowns

### 8.1 Architectural risks

- **The single-writer invariant constrains use cases.** Some workloads genuinely want concurrent writers (multi-agent systems where each agent has its own memory). The escape hatch is per-agent namespaces with the harness still mediating commit ordering, but if the user wants two agents to *share* mutable memory, we're imposing a sidecar daemon hop. This may be a wrong commitment in the multi-agent-swarm era; we should re-evaluate at month 6.
- **Bi-temporal KG cost on the hot path.** Even Mem0's single-pass extractor adds an LLM call per write. If we run contradiction-checks per episodic write, throughput tanks. The mitigation is batching contradictions into the dream cycle, but that opens a window where stale facts are retrievable. Need to measure how big that window can be in practice.
- **Postgres at >100M vectors per tenant.** pgvector HNSW is fine to ~50M; DiskANN port to pgvector is still maturing (not GA at parity yet as of May 2026). We may need to ship a Milvus/Qdrant fallback for hyperscale tenants sooner than we want.
- **Receipt signing key management.** Every install needs an Ed25519 key. Key rotation, compromise recovery, multi-install federation key directory — none of this is fun, all of it is necessary. Probably the single biggest engineering tax of the provenance commitment.

### 8.2 Empirical unknowns

- **Does observational summarization (Mastra OM) generalize beyond chat to long-running agentic workloads (Terminal-Bench style)?** Mastra's benchmarks are chat-dominated. Letta Code wins Terminal-Bench with explicit paging, not observational compression. Open question whether OM-style works for code agents.
- **Skill metabolism telemetry**. We don't yet know what "skill failure rate" thresholds produce healthy GC. Voyager's curriculum gives this naturally in a game environment; in open-ended agent workloads it's harder.
- **Are signed receipts something users actually demand?** Enterprises in regulated industries do. Solo devs absolutely don't. The risk is over-investing in provenance and losing the developer mindshare race to systems that skip it.
- **Eval gaming**. LongMemEval is saturating (94.87% is approaching the noise floor); the next benchmark wars will be on BEAM-10M and beyond. Need to commit to whichever benchmark the field converges on, not the one we picked at launch.

### 8.3 Things to watch in the field

- Anthropic's Memory MCP server evolution. If Anthropic ships a strong opinionated memory MCP, it raises the floor everyone has to clear.
- OpenAI's persistent threads + memory tools. Closed surface, but sets user expectations.
- The Graphiti license. If Zep changes Graphiti's license away from Apache 2.0, the open-source TKG landscape shifts overnight.
- DiskANN-in-pgvector GA. Materially changes the substrate decision when it lands.
- Mem0's enterprise tier — they're rumored to be adding receipts/audit logs. If they ship them well, our provenance moat narrows.

---

## Architectural commitments (the TL;DR)

If this report converges to ten decisions for Ditto to commit to in the next two weeks:

1. **Postgres + pgvector + tsvector substrate.** SQLite+sqlite-vec for embedded. No bespoke storage.
2. **Five typed memory slots**: Working, Episodic, Semantic, Procedural, Reflective. Each with explicit lifecycle.
3. **Bi-temporal model** (`t_created/t_expired/t_valid/t_invalid`) on Semantic and Reflective. Episodic is immutable.
4. **Single-writer harness boundary.** Agents emit events; harness commits. No multi-writer races.
5. **Content-addressed, Ed25519-signed receipts** for every write. Hash-chained log.
6. **Hybrid retrieval**: BM25 + pgvector HNSW → RRF → ColBERT-class rerank → KG hop. Three cost-aware modes.
7. **Two-cadence consolidation**: online (single-LLM-call Mem0-style per N events) + dream cycle (Observer/Reflector + ADM-style counterfactual verification).
8. **Skill metabolism**: procedural memory has lifecycle, deprecation, GC.
9. **RLS-by-default multi-tenancy.** Source isolation. Federated query for multi-region.
10. **Eval-as-product**: ship LongMemEval, BEAM, Ditto-Provenance-Bench, Ditto-Isolation-Bench, crash-consistency suite in-tree. Regression gates on every PR.

The combined Pareto-better claim Ditto can make:

> Better LongMemEval than Hermes/Openclaw/Openhuman. Better temporal correctness than Mem0. Better provenance than Zep. Better crash-consistency and multi-tenancy than MemPalace. Better skill lifecycle than gbrain. Same Postgres substrate as gbrain (proven), without the schema-migration treadmill. Same MCP surface as Anthropic's Memory tool, with first-class signed receipts on top.

---

## Sources

Academic:
- [LongMemEval (Wu et al., ICLR 2025)](https://arxiv.org/abs/2410.10813)
- [LoCoMo (Maharana et al.)](https://arxiv.org/abs/2402.17753)
- [Zep / Graphiti (Rasmussen et al., 2025)](https://arxiv.org/abs/2501.13956)
- [A-MEM (Xu et al., NeurIPS 2025)](https://arxiv.org/abs/2502.12110)
- [MemGPT (Packer et al.)](https://arxiv.org/abs/2310.08560)
- [Mem0 (production-ready scalable memory)](https://arxiv.org/abs/2504.19413)
- [BEAM benchmark (ICLR 2026)](https://arxiv.org/pdf/2510.27246)
- [MemBench (Tan et al., ACL 2025)](https://arxiv.org/abs/2506.21605)
- [Generative Agents (Park et al.)](https://ar5iv.labs.arxiv.org/html/2304.03442)
- [MemoryBank (Zhong et al., AAAI 2024)](https://arxiv.org/abs/2305.10250)
- [Active Dreaming Memory](https://engrxiv.org/preprint/download/5919/9826)
- [Sleep-Consolidated Memory](https://www.emergentmind.com/papers/2604.20943)
- [Voyager (Wang et al.)](https://arxiv.org/abs/2305.16291)
- [Immutable agent memory / Merkle automata](https://arxiv.org/abs/2506.13246v1)
- [Hindsight is 20/20](https://arxiv.org/html/2512.12818v1)

Product / engineering writeups:
- [Mem0 token-efficient memory algorithm](https://mem0.ai/blog/mem0-the-token-efficient-memory-algorithm)
- [Mem0 research](https://mem0.ai/research)
- [Mastra Observational Memory](https://mastra.ai/research/observational-memory)
- [Hindsight on BEAM](https://hindsight.vectorize.io/blog/2026/04/02/beam-sota)
- [Hindsight vs Supermemory](https://vectorize.io/articles/hindsight-vs-supermemory)
- [The Case Against External Vector DBs](https://hindsight.vectorize.io/blog/2026/05/12/case-against-external-vector-dbs-agent-memory)
- [Atlan: Zep vs Mem0](https://atlan.com/know/zep-vs-mem0/)
- [Vectorize: Mem0 vs Zep (2026)](https://vectorize.io/articles/mem0-vs-zep)
- [Best AI Agent Memory Frameworks 2026](https://atlan.com/know/best-ai-agent-memory-frameworks-2026/)
- [LinkedIn's Cognitive Memory Agent (InfoQ)](https://www.infoq.com/news/2026/04/linkedin-cognitive-memory-agent/)
- [Cloudflare Agent Memory](https://blog.cloudflare.com/introducing-agent-memory/)
- [Anthropic Memory Tool](https://platform.claude.com/docs/en/agents-and-tools/tool-use/memory-tool)
- [LangGraph long-term memory](https://docs.langchain.com/oss/python/langgraph/memory)
- [Letta concepts](https://docs.letta.com/concepts/memgpt/)
- [MemMachine on LoCoMo](https://memmachine.ai/blog/2025/09/memmachine-reaches-new-heights-on-locomo/)
- [ByteRover on LongMemEval](https://www.byterover.dev/blog/benchmark_ai_agent_memory_real_production_byterover_top_market_accuracy_longmemeval)
- [Mem0: State of AI Agent Memory 2026](https://mem0.ai/blog/state-of-ai-agent-memory-2026)
- [ContextSubstrate (content-addressed agent runs)](https://github.com/scalefirstai/ContextSubstrate)
- [Microsoft agent-governance-toolkit verifiable receipts](https://github.com/microsoft/agent-governance-toolkit/issues/1499)

Substrate / retrieval:
- [Tiger Data: multi-tenant RAG on Postgres](https://www.tigerdata.com/blog/building-multi-tenant-rag-applications-with-postgresql-choosing-the-right-approach)
- [Tiger Data: HNSW vs DiskANN](https://www.tigerdata.com/learn/hnsw-vs-diskann)
- [ParadeDB: Reciprocal Rank Fusion](https://www.paradedb.com/learn/search-concepts/reciprocal-rank-fusion)
- [Cross-Encoders, ColBERT, LLM rerankers practical guide](https://medium.com/@aimichael/cross-encoders-colbert-and-llm-based-re-rankers-a-practical-guide-a23570d88548)
- [DuckDB VSS](https://duckdb.org/2024/05/03/vector-similarity-search-vss)
- [VectorChord: Hybrid search with native BM25](https://docs.vectorchord.ai/vectorchord/use-case/hybrid-search.html)
