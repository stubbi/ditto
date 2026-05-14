# What Deployed AI Products Actually Do for Memory — A Forensic Survey for Ditto

*Compiled 2026-05-14. Skeptical, source-grounded. Marketing claims are flagged; engineering writeups are weighted higher.*

---

## TL;DR (2026 snapshot)

The dominant production pattern is **not** a fancy memory system. It is:

1. **Filesystem-as-memory** (markdown files in a repo or sandbox), surfaced via tools or auto-loaded prompt prefixes.
2. **A semantic retrieval layer** over messages/documents/code, increasingly built on **temporal knowledge graphs** rather than raw vector stores.
3. **Aggressive context compaction / self-summarization** for long-horizon agents.
4. **Subagents with fresh contexts** for scope isolation.
5. **Tenant- or project-scoped partitions** for governance, not because they are technically optimal.

Almost nobody at scale runs a single global vector database as their memory. The "vector DB = memory" mental model dominated 2023–24 papers but the production winners of 2025–26 (Anthropic, Augment, Glean, Cursor, Replit) all reach for filesystem + graph + compaction.

---

## 1. Coding Agents Memory Landscape

### 1.1 Cursor (Composer / Agent Mode / Cursor 2.0)

Cursor's memory story for coding is the most documented in production. Three layers:

- **Codebase index.** A Merkle-tree of file/directory hashes is computed locally; only diverging branches are re-chunked and re-embedded. Embeddings are cached per chunk content, so unchanged chunks never get re-embedded across users or sessions. Within an org, Cursor reports clones average ~92% similarity, so they **share indexes across teammates** — cutting time-to-first-query from hours to seconds on large repos. Semantic search alone is reported to lift agent accuracy by ~12.5% [1, 2].
- **Composer self-summarization (Cursor 2.0, late 2025 → 2026).** Composer was post-trained via RL to summarize *its own* trajectory as it approaches a fixed context budget, then continue. This is trained behavior, not a wrapper. It enables "hundreds of actions" within a finite context window [3].
- **Subagents.** Parallel exploration of the codebase happens in subagents, each with their own fresh context window and the best-fit model. The main agent only sees their condensed findings [1].

### 1.2 Cognition Devin

Devin's interesting bits are operational, not architectural:

- They run a **scratchpad + state log**: when Devin works on a long task, it writes its own plan and intermediate observations to disk, then is explicitly directed to re-read them. Cognition's "Rebuilding Devin for Sonnet 4.5" post is candid that they had to **tell the agent to use its memory** — implicit memory tools failed [4].
- **Feedback loops via short scripts/tests** are favored over long-running uninterruptible execution. The agent writes a test, runs it, ingests output — short, verifiable units replace long reasoning chains.
- **"Context anxiety"** was named as a real production failure: as Devin approached the model's context limit, it cut corners. The fix was to expose a *larger* context window than they actually used, capped programmatically below the boundary so the model never "felt" the wall [5].

### 1.3 Replit Agent

Replit publishes more than most about their multi-agent harness:

- **Checkpointing via git commits at every major step.** Memory is partly "the repo at time T". Users can roll back. This is the most underrated production memory primitive in the survey: durable, debuggable, structured [6].
- **Trajectory compaction with LLM summarization** when context grows. Reliability drops in later steps, so they make rollback first-class instead of fighting decay [6].
- **Scope-isolated subagents** with minimum tools — the same pattern as Cursor and Anthropic.

### 1.4 Claude Code / Codex / anthropics/skills

Claude Code is the cleanest example of **"filesystem is the database"**:

- Two complementary memory systems:
  - **`CLAUDE.md` / `AGENTS.md`** — human-written, loaded at the start of every session. Plain markdown. Project-, user-, and org-level files are concatenated [7, 8].
  - **Auto-memory** — Claude writes notes to itself during sessions (build commands, debugging patterns, style preferences). Stored as files. Also auto-loaded [7].
- **Subagents** have a `memory` field — a persistent directory that survives across conversations. The subagent's system prompt teaches it to read/write that directory. Each subagent thus owns its own personal "drawer" of accumulated lessons [9].
- **Skills** (`anthropics/skills` repo) are folders of instructions + scripts + resources, lazily loaded by Claude. Not strictly memory, but they're the same primitive: a directory of files Claude consults [10].
- **Anthropic's "Effective context engineering" post** distills the production discipline: *the smallest set of high-signal tokens*. Three techniques: compaction, tool-result clearing, persisted notes. They are explicit that memory's purpose is to **bridge fresh subagent invocations**, not to store everything ever seen [11].

### 1.5 Augment Code

Augment is the most aggressive on the **"context engine as a service"** thesis. Architecture per their own writeups:

- **Three explicit memory layers**: Intent (task spec, invariants), Environment (live codebase + dependency graph, claimed at 400k+ files), System Memory (cross-session patterns, decisions, conventions) [12].
- **Context Lineage** — they index the *commit history* as part of context, not just the snapshot. This is a real differentiator: "evolution-aware" retrieval can answer "why was this written like this?" not just "what does it do?" [13].
- Available now as an **MCP server**, decoupled from Augment's own IDE, plugging into Cursor/Claude Code/Zed/Copilot. This is the productization of the context engine itself — an interesting tell about where the moat actually sits.

### 1.6 Sourcegraph Cody / Amp

Cody pioneered **code-graph-grounded RAG**: a precomputed symbol/reference graph plus semantic embeddings, with multi-repo and org-wide context windows up to 1M tokens. Sourcegraph rebranded Cody → Amp for the agentic SKU, but the substrate is unchanged: code graph + RAG, now driving a multi-step agent loop [14].

### 1.7 GitHub Copilot — Spaces, Workspace, Memory

Copilot has three distinct memory surfaces:

- **Session context** — chat history within one conversation.
- **Spaces** — user-curated bundles of repos, PRs, issues, transcripts, files. The user explicitly assembles context. Attached repos are RAG-searched; attached files are pinned into every prompt [15].
- **Copilot Memory** — GitHub-hosted, cross-surface (cloud agent, code review, CLI). Distinct from the local VS Code memory tool. Bridges across surfaces but stays scoped to a repo or org [16].

### 1.8 Open-source / Editor agents (Aider, Continue, opencode, Zed AI)

These intentionally **don't ship memory**. Memory is delegated to MCP servers (Mem0, Letta, Graphiti, codebase-memory-mcp, etc.). Zed is the cleanest example: it implements the Agent Client Protocol and treats memory as someone else's MCP responsibility [17]. This is the "memory as substrate" market that Ditto is targeting.

---

## 2. Personal AI Memory Landscape

### 2.1 ChatGPT Memory

OpenAI's reverse-engineered architecture (and recent product writeups) suggests a much less exotic system than was assumed:

- **Saved memories** are short fact strings injected into the system prompt — not a vector retrieval. Visible to users via the **Memory Sources** UI (rolled out across consumer plans by spring 2026) — users can see *which* memory contributed to *which* response and delete it [18, 19].
- **Past-chats retrieval** is separate: ChatGPT does (limited) retrieval over your prior conversations on demand.
- **Connectors** (Gmail, Drive, GitHub) are RAG over those sources, not memory per se. The composition is: stable injected facts + on-demand retrieval over chats + on-demand retrieval over connectors.
- **Projects** are explicitly partitioned: project memory does not bleed into the main chat or other projects [20].

The big takeaway: OpenAI did **not** build a clever vector store. They built a *summarization → small-fact-list → prompt injection* pipeline, then added source attribution because deletion/audit is a UX requirement, not just a privacy one.

### 2.2 Claude.ai Memory / Projects / Managed Agents

Anthropic's three memory surfaces in 2026:

- **Consumer memory** (all tiers since March 2026) — automatically remembers preferences and ongoing projects. Similar shape to ChatGPT's [21].
- **Projects** — like ChatGPT projects, partitioned context.
- **Memory Tool / Memory MCP** (developer-facing) — Claude can CRUD files in `/memories`. The on-disk format is files, exportable via API and inspectable in the Console. Used in production by Netflix, Rakuten, Wisedocs, Ando — Anthropic claims 97% first-pass error reduction, 30% speed-up in document workflows for early adopters (marketing — directionally credible, not independently verified) [22, 23].

The architectural choice — **files-as-memory exposed via tool calls** — is the same as Claude Code's. Anthropic is converging on one substrate across products.

### 2.3 Perplexity Spaces / Memory

Perplexity is the most explicit **"memory layer is decoupled from the model"** product. You can switch GPT-5.5/Claude 4.7/Gemini 3 mid-session and memory persists. Backed by **Vespa** (not Pinecone, not a custom vector DB) for low-latency multi-tenant retrieval at millions-of-users scale. February 2026 memory upgrade: recall 77% → 95%, with *fewer* stored memories (better dedup/consolidation, not more data) [24, 25].

This is a strong signal: at consumer scale, **consolidation > accumulation**.

### 2.4 Character.AI

Despite being the largest deployed conversational-memory product (~20M MAU), the actual memory architecture is **session-buffer + persona prompt**. There is no persistent long-term memory per user-character pair. The product's biggest user complaint for two years has been "the bot forgets" [26]. Why hasn't this been fixed at scale? Two real reasons: (a) statelessness is what makes per-conversation cost tractable; (b) safety/moderation is harder when you have to redact across long histories.

This is a cautionary tale for Ditto: **the cost and safety dimensions of long memory are the actual blockers, not the engineering**.

### 2.5 Mem.ai, Reflect, MyMind

These are notes-first products with an AI surface, not memory-first. Mem.ai is the only one that genuinely tried to build an "AI thought partner" around an opinionated memory model; adoption was middling. The lesson: users don't want a separate "memory app"; they want memory inside the app they already use.

### 2.6 Pi / Replika

Pi (now subsumed into Microsoft after the Inflection deal) and Replika both rely on **summarized rolling profiles** — a maintained natural-language description of the user that gets re-injected. No public engineering writeups; behavior in the wild suggests window-of-summaries plus occasional fact extraction. Replika notoriously had memory-rewriting incidents in 2023 when they reset character personas — a real production hazard for emotional-relationship products.

---

## 3. Enterprise Memory Landscape

### 3.1 Glean

Glean is the closest thing to a "production reference architecture" for enterprise memory:

- **Per-tenant knowledge graph**, built by 100+ connectors crawling enterprise SaaS in real time [27].
- **Two graph layers**: enterprise graph (projects, people, products) and **personal graph** (your own behavior, recent docs, frequent collaborators). The personal graph is what makes results feel relevant [28].
- **Signals beyond direct edges**: document popularity, person-to-person collaboration, location, dept affinity. These are *ranking* features, not just retrieval [29].
- **Memory scope**: tenant-isolated, never shared cross-customer, and explicitly designed to remember *patterns of work* (tool sequences, workflows) rather than store sensitive content. This is a key design constraint Ditto should learn: the unit of memory at the enterprise tier is "how" the org works, not "what" it knows [27].

### 3.2 Microsoft 365 Copilot

- **Microsoft Graph** is the substrate. There is no separate memory store; the graph is the memory. Permissions are enforced via existing M365 role-based access — the Copilot can only see what the user can see [30].
- **Personalization memory** (2026) is layered on top: preferences, working styles. Tenant admins can disable it tenant-wide or per-user via PowerShell / Graph API [31].
- Stateless LLM + tenant-scoped semantic index. Pattern: **identity-and-permission-aware retrieval**, not a memory database.

### 3.3 Slack AI, Notion AI, Linear

- **Notion AI Agent 3.0/3.2** (late 2025 → early 2026): uses Notion pages and databases themselves as the memory substrate. Up to 50 pages in a single context window after the Jan 2026 upgrade. Custom Instructions live in the workspace. The Slack/Asana/Jira/Drive connectors give cross-tool synthesis [32].
- **Linear** is interesting because it exposes itself as an MCP server; many agents read sprint/issue state through it but Linear itself isn't running a memory system — issues *are* the memory.
- **Slack AI** does channel/conversation summarization and now ships agent surfaces; memory is conversation-scoped + workspace-scoped.

### 3.4 Salesforce Agentforce / Einstein

- The memory is **Data Cloud** — unified customer profiles built from CRM, billing, support, web data. Agents don't search for context; they're "born with it" because every agent runs against the unified profile [33].
- Context window is currently capped at 65,536 tokens with data masking on (Einstein Trust Layer constraint).
- **Spring '26**: Intelligent Context, Agentforce Voice, Agent Script went GA. Agent-to-agent handoff with full context retention is now MCP/A2A-based.

### 3.5 Harvey (legal)

Harvey announced "Memory" in January 2026 as a co-build with law firms. Four layers, explicitly designed around how law firms actually work [34]:

- **Personal lawyer memory** — your style, your matter context.
- **Matter-specific memory** — bound to an engagement, retained per firm policy.
- **Institutional memory** — firm-wide processes, precedents, approved templates.
- **Client-institution memory** — the relationship between firm and client, codified.

The interesting bit isn't technical — it's the **scoping taxonomy**. Ditto should steal this directly: most enterprise memory products lump everything into one "workspace", which is wrong. Memory has natural scopes (me, this matter, this client, the firm), each with different retention, sharing, and audit semantics.

### 3.6 Sierra (CX)

Sierra's **Agent Data Platform (ADP)** unifies unstructured (calls, chats, emails) with structured (CRM, billing, inventory). The memory model is conversation-as-feature-of-customer-profile. "Build once, deploy everywhere" across channels (chat, voice, SMS, contact center). Every conversation enriches the agent's view of the customer [35]. Same shape as Salesforce, just CX-flavored.

---

## 4. Agent Framework Memory Primitives

| Framework | Memory model | Notes |
|---|---|---|
| **LangGraph** | Short-term via execution state; long-term left to user (vector DB, etc.) | Maximally flexible, minimally opinionated. You bring the store. |
| **CrewAI** | Built-in short/long/entity memory + knowledge sources | Opinionated. Easier to start, harder to swap. |
| **AutoGen** | Conversation history + optional retrievers | Sub-agent architecture is its memory pattern. |
| **LlamaIndex** | Indexes + Query Engines as first-class primitives | RAG-native; memory is "an index over chat history". |
| **Letta (MemGPT)** | OS-inspired memory hierarchy: core (in-context) / message buffer / archival/recall | Production framework; runs as a service behind REST. Most mature open architecture. [36] |
| **Mem0** | Extract → consolidate → retrieve. Optional graph variant. | LoCoMo 91.6, LongMemEval 93.4, BEAM 64.1/48.6 (1M/10M tokens). p95 ~91% lower than full-context. ~7k avg tokens/retrieval vs 25k+ for naive RAG. Async-by-default since v1.0 [37, 38]. |
| **Zep / Graphiti** | Temporal knowledge graph; every fact has a validity window | P95 retrieval ~300ms. Outperformed MemGPT on DMR benchmark. Open-source Graphiti is the engine; Zep is the managed service [39, 40]. |
| **Anthropic Memory MCP** | Knowledge graph stored in JSONL on disk + Filesystem MCP for direct file access | The "official" Anthropic answer is *write to files* — the Memory MCP server is a thin knowledge-graph layer over JSONL [41]. |

The framework world has bifurcated: **graph-backed temporal memory** (Zep, Graphiti, Mem0+graph, Letta+graph) on one side, **plain filesystem + RAG** (Anthropic's stance, Cursor, Claude Code) on the other. The graph camp wins on benchmarks; the filesystem camp wins on debuggability, governance, and audit.

---

## 5. What Production Systems Converge On

After surveying ~30 deployed systems, the convergence is striking:

1. **Filesystem or document store as primary memory substrate.** Claude Code, Cursor (caches/embeddings on disk), Replit (git commits), Notion (pages-as-memory), Augment (codebase index), Anthropic Memory MCP (JSONL), Harvey ("matter folders"). The "memory database" is increasingly a misnomer.
2. **Aggressive consolidation and decay.** Perplexity shipped a recall improvement by *reducing* stored memories. Mem0's whole pitch is fewer-tokens-per-retrieval. ChatGPT exposes a sources UI specifically so users can prune.
3. **Compaction / self-summarization for long-horizon agents.** Cursor's Composer is RL-trained on this. Anthropic's context-engineering posts canonize it. Devin learned the hard way ("context anxiety").
4. **Subagent isolation.** Cursor, Claude Code, Replit, Anthropic's multi-agent research system, AutoGen — all use fresh-context subagents and only persist condensed handoffs.
5. **Scoped memory with explicit boundaries.** Project (ChatGPT, Claude), Matter/Client (Harvey), Tenant (Glean, M365), Repo (Copilot, Augment). The default-global memory is universally rejected.
6. **Identity- and permission-aware retrieval, not a flat memory pool.** Glean's tenant graph, M365 Copilot's role-based access, Salesforce's profile-scoped memory. Memory cannot be separated from "who is allowed to see this?".
7. **User-visible memory + delete.** ChatGPT Memory Sources, Claude memory inspection, Harvey's admin controls. Once you ship memory at scale, you ship a UI to manage it — otherwise users (and regulators) revolt.
8. **Decoupled from the model.** Perplexity, Mem0, Zep, Augment MCP — memory is increasingly a separate service so you can swap model providers. This is anti-lock-in pressure from buyers, and it's the structural reason a memory layer like Ditto can exist as a product.

---

## 6. What Production Systems Disagree On

Real design tensions, not marketing differences:

1. **Vector vs. graph vs. file primitives.** Zep/Graphiti/Mem0-graph argue temporal graphs win on retrieval quality. Anthropic and Cursor argue files + structured retrieval win on debuggability and on "the model already knows how to read files". This is unresolved.
2. **Write-time consolidation vs. read-time synthesis.** Mem0 consolidates on write (extract → dedup → store atomic facts). Glean/Perplexity lean read-time (assemble at query time from raw signals). Write-time is cheaper at read but lossy; read-time is expensive but lossless.
3. **Auto-memory vs. user-curated memory.** Claude Code's auto-memory writes notes without asking; OpenAI's UI is heavily user-visible-and-editable; Harvey is fully opt-in per scope. The tradeoff is friction vs. trust.
4. **Cross-session vs. cross-project memory.** ChatGPT projects partition. Claude projects partition. Harvey explicitly *bridges* across matters when wanted. Augment bridges across repos. No consensus.
5. **Implicit retrieval vs. agent-driven retrieval.** Anthropic ("the agent should explicitly read its memory") vs. Glean/M365 (memory is injected by the platform). Cognition explicitly flipped to the Anthropic view after the Devin rebuild.
6. **Decay policy.** Almost nobody publishes one. Perplexity hints at consolidation. Mem0 has TTLs. Most products quietly accumulate forever.
7. **Multi-tenant key shape.** Per-user (ChatGPT, Pi), per-user-per-project (Claude, GitHub), per-tenant-per-team (Glean), per-matter (Harvey). The grain of the "memory scope" is product-dependent and hard to change later.

---

## 7. Lessons for Ditto

### Copy these:

1. **Filesystem-first interface.** Make Ditto memory feel like a directory of files. Claude/Anthropic's "files-as-memory" pattern is winning because it's debuggable, exportable, version-controllable, and the LLM already knows how to read files. Make the on-disk format a first-class API.
2. **Temporal knowledge graph as the *index*, not the *interface*.** Steal Graphiti's validity-window idea — every fact has "true since T, superseded at T'", with confidence. But expose it through file/document APIs, not raw Cypher.
3. **Scope taxonomy borrowed from Harvey.** Personal / project / org / cross-org. Each scope has its own retention, sharing, and audit defaults. Ship this on day one — it's much harder to add later.
4. **Compaction + decay as first-class operations.** Don't just store. Promise the operator a `consolidate` and a `decay` primitive, with policies they can configure. Perplexity proved fewer/better beats more/raw.
5. **Per-tenant isolation enforced at the storage layer, not just app-level.** Glean's approach. This is what unlocks the enterprise tier.
6. **Memory sources UI.** Every fact returned cites its source memory. ChatGPT shipped this in 2026 because they had to. Ditto should ship it from day one.
7. **MCP-native.** This is non-negotiable in 2026. Cursor, Claude Code, Zed, Copilot all consume MCP. Augment's Context Engine going MCP-only is the strongest signal — the memory product is the MCP server.
8. **Decouple memory from the model.** Perplexity, Mem0, Zep all prove model-agnostic memory is what enterprise buyers actually want. The structural pitch: "We outlive your model choice."
9. **Subagent memory drawers.** Claude Code's per-subagent persistent directory is a brilliant primitive. Memory shouldn't only be user-scoped; it should be *role-scoped*.
10. **Don't fight context anxiety; design around it.** Expose effective vs. nominal budgets. Let agents see "soft limits" that pause and compact before the model panics.

### Skip these:

1. **Don't ship a notes app.** Mem.ai's lesson. Memory inside other tools >>> a standalone memory tool.
2. **Don't build a single global vector store.** It's the 2023 answer to a 2026 question. The winners use graphs, files, or both — and partition aggressively.
3. **Don't auto-accumulate without auto-decay.** Replika and Character.AI's user complaints prove the failure mode. Set decay defaults and surface them.
4. **Don't reinvent embeddings infra.** Cursor's Merkle-tree + shared-index design is publicly described and is genuinely the right shape. Build on commodity vector storage (pgvector, LanceDB, Vespa) and put the value elsewhere.
5. **Don't optimize for benchmarks alone.** LoCoMo/LongMemEval/BEAM are useful but Mem0 winning them hasn't translated to displacing filesystem-based memory in Claude Code or Cursor. The benchmarks don't measure governance, audit, debuggability, or operator trust.
6. **Don't conflate skills/instructions with memory.** Anthropic's `anthropics/skills` is *not* memory — it's procedural knowledge. Keep them separate or you'll confuse buyers.

---

## 8. What Ditto Can Do That None of Them Can

Opportunities production has left on the table:

1. **A real cross-product memory layer.** Every product above scopes memory inside its silo. Glean unifies *read* across SaaS but doesn't expose a write API for agents from elsewhere. ChatGPT can't see Claude's memory and vice versa (a 9to5Mac piece in March 2026 noted Claude finally letting free users import context from rivals — the importer is a workaround, not a standard). The actually-cross-product memory layer is missing. **Ditto can be the connective tissue.**
2. **Temporal-graph semantics with file-shaped UX.** Graphiti has the right primitive but the wrong interface for most users (raw graph). Files have the right interface but the wrong primitive (no validity windows, no conflict resolution). The synthesis — "files that know when their contents stopped being true" — is open territory.
3. **First-class memory provenance and audit.** Harvey, Glean, M365 each have *some* of this for their silo. None publish a portable audit trail across agents. With regulators increasingly asking "what did the AI know and when did it know it?", this is real product surface.
4. **Operator-grade decay and consolidation policies.** Expose these as DSL/config, not internal magic. Nobody productizes "tell me what your agent forgot last week" — it's a missing observability surface and probably a wedge into ops buyers.
5. **Memory for sub-agents as a primitive, not an afterthought.** Claude Code stumbled into this. Nobody else has it cleanly. With agent swarms going mainstream in 2026, role-scoped persistent memory is increasingly load-bearing.
6. **Conflict resolution at the fact level.** When two agents (or two humans) record contradictory facts, what happens? Production systems mostly punt: last-write-wins or both-stored. Graphiti's validity windows hint at a solution; nobody ships it as a UX primitive.
7. **A migration story.** Enterprises with existing M365/Salesforce/Glean memory want to keep it but layer Ditto on top. None of the incumbents publish stable export formats. Ditto could be the first memory layer that *ingests* from existing silos rather than asking them to be replaced.
8. **Verifiable deletion.** GDPR / "right to be forgotten" + AI memory is a coming regulatory storm. Cryptographically attestable deletion of a memory and everything it influenced is a moat-grade feature nobody offers yet.

---

## 9. Citations

1. [Cursor — Securely indexing large codebases](https://cursor.com/blog/secure-codebase-indexing)
2. [Cursor 2026: Composer, Agent Mode, MCP & Background Agent (DeployHQ)](https://www.deployhq.com/guides/cursor)
3. [Cursor — Training Composer for longer horizons (self-summarization)](https://cursor.com/blog/self-summarization)
4. [Cognition — Rebuilding Devin for Claude Sonnet 4.5](https://cognition.ai/blog/devin-sonnet-4-5-lessons-and-challenges)
5. [Compaction: The Hidden Trick That Keeps AI Coding Agents from Forgetting Everything (PracticeOverflow)](https://practiceoverflow.substack.com/p/compaction-the-hidden-trick-that)
6. [Replit Agent Case Study (LangChain Breakout Agents)](https://www.langchain.com/breakoutagents/replit)
7. [Claude Code — How Claude remembers your project](https://code.claude.com/docs/en/memory)
8. [The Complete Guide to AI Agent Memory Files (CLAUDE.md, AGENTS.md, …)](https://medium.com/data-science-collective/the-complete-guide-to-ai-agent-memory-files-claude-md-agents-md-and-beyond-49ea0df5c5a9)
9. [Claude Code — Create custom subagents](https://code.claude.com/docs/en/sub-agents)
10. [anthropics/skills GitHub repo](https://github.com/anthropics/skills)
11. [Anthropic — Effective context engineering for AI agents](https://www.anthropic.com/engineering/effective-context-engineering-for-ai-agents)
12. [Augment Code — Context Engineering: Intent, Environment, System Memory](https://www.augmentcode.com/guides/context-engineering-enhancing-agentic-swarm-coding-through-intent-environment-and-system-memory)
13. [Augment Code — Context Lineage announcement](https://www.augmentcode.com/blog/announcing-context-lineage)
14. [Sourcegraph Cody docs](https://sourcegraph.com/docs/cody)
15. [GitHub Docs — Using Copilot Spaces](https://docs.github.com/en/copilot/how-tos/provide-context/use-copilot-spaces/use-copilot-spaces)
16. [VS Code — Memory in VS Code agents](https://code.visualstudio.com/docs/copilot/agents/memory)
17. [Zed AI Code Editor — Overview](https://zed.dev/docs/ai/overview)
18. [OpenAI Help — Memory FAQ](https://help.openai.com/en/articles/8590148-memory-faq)
19. [How ChatGPT Memory Works, Reverse Engineered (LLMRefs)](https://llmrefs.com/blog/reverse-engineering-chatgpt-memory)
20. [ChatGPT Features 2026: Projects, Memory, Agent, Sora (Suprmind)](https://suprmind.ai/hub/chatgpt/features/)
21. [9to5Mac — Free Claude users can now use memory and import context from rivals](https://9to5mac.com/2026/03/02/free-claude-users-can-now-use-memory-and-import-context-from-rivals/)
22. [Anthropic — Memory tool docs](https://platform.claude.com/docs/en/agents-and-tools/tool-use/memory-tool)
23. [9to5Mac — Anthropic updates Claude Managed Agents](https://9to5mac.com/2026/05/07/anthropic-updates-claude-managed-agents-with-three-new-features/)
24. [How Perplexity Built an AI Google (ByteByteGo)](https://blog.bytebytego.com/p/how-perplexity-built-an-ai-google)
25. [Perplexity Memory: What It Remembers (Supermemory)](https://supermemory.ai/blog/how-perplexity-memory-works/)
26. [Character.AI overview (EmergentMind)](https://www.emergentmind.com/topics/character-ai-c-ai)
27. [Glean — The Glean knowledge graph](https://www.glean.com/resources/guides/glean-knowledge-graph)
28. [Glean — Enterprise Graph product](https://www.glean.com/product/enterprise-graph)
29. [Glean — How Glean search works](https://www.glean.com/resources/guides/how-glean-search-works)
30. [Microsoft Learn — M365 Copilot architecture](https://learn.microsoft.com/en-us/copilot/microsoft-365/microsoft-365-copilot-architecture)
31. [Microsoft Learn — Copilot personalization and memory](https://learn.microsoft.com/en-us/copilot/microsoft-365/copilot-personalization-memory)
32. [TechCrunch — Notion just turned its workspace into a hub for AI agents](https://techcrunch.com/2026/05/13/notion-just-turned-its-workspace-into-a-hub-for-ai-agents/)
33. [Salesforce Agentforce architecture (MindStudio)](https://www.mindstudio.ai/blog/salesforce-agentforce-architecture-slack-data-agents)
34. [Harvey — Never Start From Scratch Again With Memory in Harvey](https://www.harvey.ai/blog/memory-in-harvey)
35. [Sierra — Introducing the Agent Data Platform](https://sierra.ai/blog/agent-data-platform)
36. [Letta — Research background (MemGPT memory hierarchy)](https://docs.letta.com/concepts/letta/)
37. [Mem0 — Building Production-Ready AI Agents with Scalable Long-Term Memory (arXiv 2504.19413)](https://arxiv.org/abs/2504.19413)
38. [Mem0 — Benchmarking Mem0's token-efficient memory algorithm](https://mem0.ai/research)
39. [Zep — Temporal Knowledge Graph Architecture for Agent Memory (arXiv 2501.13956)](https://arxiv.org/abs/2501.13956)
40. [getzep/graphiti GitHub repo](https://github.com/getzep/graphiti)
41. [Knowledge Graph Memory MCP Server (modelcontextprotocol/servers)](https://github.com/modelcontextprotocol/servers/tree/main/src/memory)
42. [Anthropic — Effective harnesses for long-running agents](https://www.anthropic.com/engineering/effective-harnesses-for-long-running-agents)
43. [Anthropic — Multi-agent research system](https://www.anthropic.com/engineering/multi-agent-research-system)
44. [Anthropic — Scaling Managed Agents](https://www.anthropic.com/engineering/managed-agents)
45. [Project Astra — Google DeepMind](https://deepmind.google/models/project-astra/)
