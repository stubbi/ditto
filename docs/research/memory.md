# Memory research — synthesis

*Updated 2026-05-14 after a four-vector deep-research pass.*

This is the entry point. The research lives in five sub-documents under [`memory/`](./memory/):

- [`landscape.md`](./memory/landscape.md) — Round 1: the 2025–2026 landscape (Mem0, Zep/Graphiti, Letta, Mastra OM, MemPalace, gbrain, Hindsight, benchmarks, citations)
- [`arxiv.md`](./memory/arxiv.md) — frontier research papers from late 2025–Q2 2026 (RL-trained memory ops, hypernetwork LoRAs, surprise-gated writes, SCITT receipts, late-interaction retrieval, ~80 arXiv cites)
- [`biology.md`](./memory/biology.md) — neuroscience grounding (CLS, hippocampal indexing, reconsolidation, schema-gated consolidation, replay, predictive coding, forgetting-as-feature)
- [`production.md`](./memory/production.md) — forensic survey of ~30 deployed products (Cursor, Devin, Replit, Claude Code, Augment, ChatGPT/Claude memory, Glean, Harvey, M365, agent frameworks)
- [`trending.md`](./memory/trending.md) — OSS velocity, practitioner pains/asks, surprise findings (MemPalace scandal, embedding de-emphasis, swyx/Karpathy commentary, EU AI Act timeline)

What follows is the synthesis: what the four vectors agree on, what they disagree on, and what changes about Ditto's architecture as a result. The architectural commitments live in [`../architecture/memory.md`](../architecture/memory.md) (v2).

---

## What the four vectors agree on

Six findings are over-determined — they show up across multiple vectors independently. These are the load-bearing claims for Ditto's design.

### 1. Filesystem-as-memory has won in production; embeddings are getting de-emphasized

**Production:** Claude Code, Anthropic Memory MCP, Cursor (file caches + Merkle-tree index), Replit (git commits as memory), Notion (pages-as-memory), Augment (codebase index), Harvey (matter folders), ChatGPT (saved-memories injected as text into the system prompt — not vector retrieval).

**Trending:** engram (single Go binary, SQLite+FTS5), Memvid (single `.mv2` file), ByteRover (human-readable markdown, *zero* vector DB, beats Hindsight by 14.3 pts), Supermemory's 99% SOTA experimental flow *ditched vector embeddings entirely*. Karpathy's Sequoia 2026 "agent wiki" pattern: conversations → daily logs → wiki → injected back.

**Implication for Ditto.** Embeddings are *one signal*, not THE signal. Files are a first-class memory primitive, not just a UX skin. The v1 architecture's Postgres-only substrate was a 2023 frame; the v2 substrate is **filesystem + temporal KG + tsvector + pgvector**, with embeddings strictly pluggable.

### 2. The "memory controller" is the missing piece — everyone wants it, no one ships it

**Trending #1 ask:** "Most memory discussions focus on storage backends, retrieval algorithms, and context injection, but the component that is consistently missing is the memory controller." (Marco, byMAR.CO, 2026)

**Arxiv:** Memory-R1 (arXiv 2508.19828), Mem-α (arXiv 2509.25911) train an explicit Memory Manager via GRPO with only 152 QA pairs and beat hand-tuned Mem0-class baselines.

**Production:** Cursor 2.0's Composer is *RL-trained* to self-summarize at context boundaries — the controller is the moat, not the store.

**Implication for Ditto.** Ditto's positioning is the controller (what to write / when to expire / what to forget / when to retrieve), not the store. Stores are pluggable; the controller policy is learned and is the IP.

### 3. Bi-temporal facts + signed receipts + verifiable deletion are about to be regulatory table stakes

**Arxiv:** SCITT (draft-ietf-scitt-architecture-22) is the emerging standard for COSE-signed Merkle-tree-anchored receipts. Microsoft Azure ships a SCITT transparency service.

**Production:** Anthropic's enterprise Managed Agents (April 2026) ship "All memory changes are logged, with audit trails for each session and agent, ability to roll back, redact." Harvey 6× task completion. Rakuten 27%/34%/97% cost/latency/error cuts.

**Trending:** EU AI Act high-risk compliance deadline is **August 2026** — automatically generated logs + post-market monitoring. Most memory systems offer no audit trail. Verifiable deletion (GDPR + AI memory) is open whitespace.

**Biology:** Trace transformation theory + reconsolidation say memories *update* and *invalidate* but don't get destroyed. Bitemporal storage is the engineered equivalent.

**Implication for Ditto.** Every write produces a SCITT-compliant receipt; the receipt chain is per-tenant Merkle-anchored; deletes are cryptographically attestable and cascade to derived records. This is in v1 already; the SCITT compliance specifics are the v2 upgrade.

### 4. Salience/surprise-gated writes; reconsolidation labile window on reads

**Arxiv:** Selective Memory write-time gating (arXiv 2603.15994), Adaptive Memory Admission Control / A-MAC (arXiv 2603.04549), Continuum Memory Architectures (arXiv 2601.09913) all propose composite write-time gates (novelty × prediction error × confidence × recency × content prior).

**Biology:** Predictive coding (Rao & Ballard 1999; Henson & Gagnepain 2010) says don't store the predictable. Reconsolidation (Nader, Schafe & LeDoux 2000) says every retrieval opens a labile window. Locus-coeruleus norepinephrine system gates encoding on salience (Mather et al. 2016).

**Trending:** Practitioner pain — stale memories that become "confidently wrong rather than just outdated." This is the predictable consequence of writing everything and never updating on retrieval.

**Production:** No deployed system does either of these. Open whitespace.

**Implication for Ditto.** Before a write, ask NC: "what did you predict?" Compare to actual. Score residual. Threshold-gate the write. On every retrieval, open a bounded labile window during which corrections from *trusted sources* (user, verified tools) rewrite the trace; close window after N turns or seconds.

### 5. Schema-gated fast consolidation; sleep-cycle / dream cycle is now standard

**Biology:** Tse, Langston, Kakeyama et al. (2007, *Science*) — rats with a pre-existing schema consolidated new facts in 24h vs. weeks without. SLIMM model (van Kesteren et al. 2012) formalized schema-gated routing. Sharp-wave ripple replay (Karlsson & Frank 2009; Yu/Liu/Frank 2024 *Science*) selects experience for memory.

**Arxiv:** SCM (arXiv 2604.20943), NeuroDream, "Learning to Forget" (arXiv 2603.14517), SleepGate — explicit NREM/REM-style background consolidation as a first-class subsystem.

**Production:** Anthropic shipped Auto-Dream + Dreaming in Managed Agents April 2026 (Harvey 6×, Wisedocs 50% review-time cut). Letta sleep-time agents. Cursor 2.0 RL-trained compaction. Convergent.

**Trending:** dream-skill (community port of Anthropic Auto-Dream), GenericAgent's L0-L4 consolidation, agentmemory's 4-tier hooks. Karpathy's "agent wiki" is the same shape.

**Implication for Ditto.** Three replay/consolidation cadences:
- **Awake ripple** (between turns, ≤200ms): salience-weighted replay of recent episodes, tag winners for next dream cycle.
- **Dream cycle** (session-close + 24h): schema-fit check on tagged episodes. Fits → fast NC commit + HC trace weakening. Novels → defer, accumulate corroboration before schema revision.
- **Long sleep** (daily/weekly): decay sweep, retrieval-induced suppression sweep, conflict detection, spaced-retrieval self-tests.

Plus **TMR cueing**: user/harness can bias what gets replayed ("think about the auth refactor tonight").

### 6. Hippocampal indexing — episodic should be a thin index over content-addressed blob storage

**Biology #1 finding:** Teyler & DiScenna (1986); Liu/Ramirez/Tonegawa (2012 *Nature*). The hippocampus does not contain the experience — it indexes into cortical patterns. Hippocampus is ~1% of cortex by volume because it's an index.

**Production:** Augment's "Context Lineage" indexes git commit history; Cursor's Merkle-tree caches embeddings per chunk; Anthropic Memory MCP stores knowledge graph as JSONL pointing at files. Same pattern.

**v1 problem:** my v1 Ditto architecture treated episodic as full content. That's the MemPalace failure mode: verbatim storage doesn't scale (cf. the Shereshevsky case study in Luria 1968 — perfect memory destroys abstraction and social function).

**Implication for Ditto.** Episodic becomes `{event_id, timestamp, sparse_key[], salience, content_hash[], context}` — pointers and sparse keys only. The raw content (transcripts, tool outputs, file diffs) lives in a **content-addressed blob store**. This makes episodic storage roughly 100× cheaper and matches both the biology and the production convergence.

---

## What the four vectors disagree on

Productive disagreements; each is a design choice Ditto must make explicitly.

### A. Vector vs. graph vs. file as primary substrate

**Vector camp:** Mem0, vector-DB-by-default in most agent frameworks. Easy, commodity.
**Graph camp:** Zep/Graphiti, Hindsight (multi-channel fusion). Best benchmarks on temporal reasoning.
**File camp:** Anthropic (Claude Code, Memory MCP, Managed Agents), Cursor, Replit, Augment. Best operator trust, debuggability, governance.

**Ditto decision:** all three, layered. **File store** holds canonical content (operator-visible, exportable, version-controllable). **Temporal KG** is the typed index (Graphiti-style validity windows, but in Postgres tables not Neo4j). **Vector index** is one signal among BM25 + tsvector + KG entity match — never the sole retrieval path. The integration surface (MCP server, in-process SDK) consumes all three through one query API.

### B. Write-time consolidation vs. read-time synthesis

**Write-time camp:** Mem0 extracts atomic facts at write time. Cheap reads, lossy.
**Read-time camp:** Glean, Perplexity (read-time graph fusion). Expensive reads, lossless.

**Ditto decision:** episodic is *always* lossless (raw content into blob store, index in episodic table). Semantic facts are extracted by the consolidator (background, not on write path) and stored in the KG. Reads default to the KG; high-stakes reads follow provenance to the blob. This is the Anthropic / Cursor compromise — fast at low stakes, lossless at high stakes.

### C. Auto-memory vs. user-curated

**Auto camp:** Claude Code's auto-memory writes without asking. ChatGPT Memory.
**Curated camp:** Harvey's per-scope opt-in. ChatGPT Memory Sources UI lets users see and prune.

**Ditto decision:** auto-write at episodic (always lossless) + user-curated promotion to semantic (the consolidator surfaces proposals, the user accepts/rejects). User-visible source attribution on every retrieval — this is now table stakes after ChatGPT shipped it.

### D. Implicit retrieval (platform-injected) vs. agent-driven (tool calls)

**Implicit camp:** Glean, M365 Copilot inject memory into the prompt.
**Agent-driven camp:** Anthropic explicitly directs the agent to read its memory. Cognition flipped to this view after the Devin rebuild ("we had to tell the agent to use its memory").

**Ditto decision:** agent-driven by default, implicit-injection optional. The agent decides when to retrieve via a metacognitive gate (RSCB-MC, arXiv 2604.27283). Cheaper, more controllable, and matches where production is converging.

---

## Three architectural deltas v1 → v2

The detailed v2 architecture is in [`../architecture/memory.md`](../architecture/memory.md). Three changes are big enough to call out here:

### Delta 1: Memory slots refactored

| v1 slot | v2 slot(s) | Why |
|---|---|---|
| Working | Working (kept) | Unchanged |
| Episodic (full content) | Episodic-index (sparse keys + pointers) + Blob-store (content-addressed) | Hippocampal indexing; production convergence; ~100× cheaper |
| Semantic (bi-temporal facts) | NC-graph (typed property graph, bi-temporal) + NC-doc (compiled per-entity Markdown pages) | Files-as-memory has won; NC-doc is the operator-facing view |
| Procedural (skills) | Procedural (kept, with metabolism) | Unchanged |
| Reflective | Reflective (kept) | Unchanged |
| — | SDM-assoc side channel (optional) | Sparse high-dim hash for pattern completion from partial cues |

### Delta 2: Memory controller is the moat (not the store)

Add an explicit **MemoryController** subsystem:

- **Write-time policy:** surprise-gated writes; salience scoring; schema-fit prediction.
- **Read-time policy:** metacognitive retrieval gate (RSCB-MC); cost-aware mode selection (cheap/standard/deep).
- **Consolidation policy:** awake ripples + dream cycle + long sleep; schema-fit gating; skill metabolism.
- **Decay policy:** retrieval-induced suppression; budgeted forgetting; bitemporal supersession.

The policy is **learned** (Memory-R1 / Mem-α pattern, GRPO, 152-QA-pair training set is sufficient as a starting point). Stores are pluggable backends behind the policy.

### Delta 3: Scope taxonomy refined (Harvey's 4-layer)

v1 had Org / Tenant / Workspace / Agent. v2 inserts **Matter** and **Institutional** between Workspace and Tenant:

```
Org
└── Tenant                 isolation, encryption, audit boundary
    ├── Institutional      org-wide processes, conventions, approved templates
    └── Workspace          project / RBAC scope
        ├── Matter         per-engagement, retention-policied
        ├── Agent          runtime instance
        └── Subagent       role-scoped drawer (Claude Code pattern)
```

Each scope has its own retention, sharing, and audit defaults. This is much harder to retrofit than to build in, and Harvey's case study shows the legal/compliance market won't compromise on it.

---

## What we are not adopting (and why)

- **Parametric model editing (ROME / MEMIT) as primary memory.** arXiv 2502.11177 — degrades after 10-40 edits. Possible for "stable identity facts the user keeps re-asserting"; not a primary substrate.
- **Per-user growing LoRA stacks.** LoRA doesn't prevent catastrophic forgetting under continual updates.
- **RAFT / RA-DIT fine-tuning for frontier-API deployments.** Frontier models are already retrieval-aware enough; the delta is marginal.
- **Naive 1M-token long-context as a memory replacement.** ~30-60× latency, ~1250× cost; only Gemini 3 Deep Think holds quality. Long-context is in-session working memory.
- **Full TEM / cognitive-map architectures.** Beautiful research; no production-ready system. Track, don't build.
- **A standalone notes app.** Mem.ai's lesson. Memory lives inside other tools.
- **Single global vector store.** 2023 frame; 2026 winners partition.
- **Embeddings as load-bearing primary retrieval.** Supermemory and ByteRover proved you can ditch them. Keep optional.

---

## Eval surface additions

The 4 new benchmarks identified by the research, to add to the in-tree eval suite (see [`../../eval/`](../../eval/)):

| Benchmark | Source | What it tests | Status |
|---|---|---|---|
| **MemoryAgentBench** | ICLR 2026 (HUST-AI-HYZ) | 4 competencies: accurate retrieval, test-time learning, long-range understanding, selective forgetting | forthcoming |
| **AMA-Bench** | ICLR 2026 Memory Agent workshop | Non-dialogue agent-environment streams; synthetic + real trajectories | forthcoming |
| **Ditto-DRM-Bench** | Ditto-original | Roediger-McDermott false-memory test: ensure consolidator doesn't fabricate schema-consistent details | forthcoming |
| **Ditto-Deletion-Bench** | Ditto-original | Verifiable deletion + cascade — given a fact F, prove all derived records are gone after delete | forthcoming |

Plus the existing **Provenance-Bench** and **Isolation-Bench**. The MemPalace scandal lesson: always publish matched-conditions BM25 baseline on the same haystack at full corpus scale, never on a 50-session subset.

---

## What's next

1. Update [`../architecture/memory.md`](../architecture/memory.md) — v2 with the deltas above (this commit).
2. Scaffold `ditto-memory` Rust crate against the v2 architecture (next commit, pending review of this synthesis).
3. Add MemoryAgentBench / AMA-Bench / DRM-Bench / Deletion-Bench runners to the eval harness.
4. Re-run the harness against the stub and (once it exists) the Ditto backend.

The four sub-documents under [`memory/`](./memory/) are the citation-grounded source-of-truth; revisit them when specific design decisions are contested.
