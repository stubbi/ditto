# Biologically Plausible Memory Architectures for the Ditto Agent Harness

A deep technical report translating ~80 years of memory neuroscience into concrete design choices for `github.com/stubbi/ditto`, with comparisons to Mem0, Zep, Letta/MemGPT, Mastra Observational Memory (OM), MemPalace, GBrain, and Hindsight.

Date: 2026-05-14. Opinionated. Cites specific experiments.

> Reading note on Ditto: the public README of the only "ditto" repos I could locate (yoheinakajima/ditto, ditto-assistant) does not document a memory architecture beyond a vector store, so this report treats Ditto as a clean-slate design target and proposes a memory layer rather than critiquing an existing one. If `stubbi/ditto` has a different shape, the architecture sketch in §4 should be read as a target rather than a diff.

---

## 1. Top 7 biological principles Ditto should adopt (ranked by leverage)

These are ranked by *expected delta over the current SOTA agent memory stack* (Mem0 / Zep / Letta / Mastra OM / MemPalace / Hindsight). High-leverage = the systems above either don't do this, do it incidentally, or do it wrong.

1. **Hippocampal indexing + CLS separation.** Have a thin, fast, sparse episodic *index* and a slow, dense, generalizing semantic *store*. Most current systems either collapse them (MemPalace: verbatim only; Mem0: extracted facts only) or layer them incoherently. This is the single biggest architectural commitment.
2. **Reward-tagged awake replay (selective consolidation).** During quiet moments — between turns, between tasks, before sleep cycles — replay only the episodes the agent *tagged* as surprising or rewarded. Mastra OM has the closest analogue (Observer→Reflector), but it consolidates on a token-pressure trigger, not a salience trigger. Biology says: tag at encoding, replay later, prioritize by reward-prediction error.
3. **Schema-gated fast consolidation.** New facts that fit existing schemas consolidate in one shot; novel facts go through the slow loop. Tse et al. (2007) demonstrated 24-hour consolidation when a schema pre-exists vs. weeks otherwise. No current agent system makes the schema-fit decision explicit.
4. **Reconsolidation: every retrieval is a write opportunity.** When the agent retrieves a memory and the retrieved memory is contradicted, updated, or used in a new context, write back the modification. Mem0 partially does this (it overwrites stale facts); Letta exposes it as a tool; none implement the labile-window discipline (a bounded window after retrieval in which corrections are accepted).
5. **Surprise/prediction-error gated encoding.** Don't store the predictable. Encode when the agent's prior over what would happen disagrees with what happened. This is the cleanest biological argument against MemPalace-style "store everything verbatim" — biology stores almost *nothing* verbatim.
6. **Forgetting as a feature, not a failure.** Retrieval-induced forgetting + a Shereshevsky-style explicit budget on accumulated detail. Most agent systems treat forgetting as something they hope happens via summarization. Make it an explicit competitive process.
7. **Episodic ↔ future thinking symmetry.** The same store that supports "what did we try last time?" should support "what's likely to happen if we try X?" via recombination of trace fragments. This single principle reframes memory from a read-only log into a generative simulator.

The remaining 13 principles below all matter, but each captures less marginal leverage than these seven.

---

## 2. Deep dive on the 20 topics

### 2.1 Complementary Learning Systems (CLS)

**Biology.** McClelland, McNaughton & O'Reilly (1995, *Psych Rev*) argued the brain has two systems because a single network can't simultaneously (a) learn one-shot from individual experiences and (b) extract slow statistical regularities without catastrophic interference. The hippocampus uses sparse, pattern-separated codes (CA3/DG) for fast episodic binding; neocortex uses overlapping distributed codes for slow generalization. Recent work (Kumaran, Hassabis & McClelland 2016 *TiCS*; Sun et al. 2023 on CLS in deep nets) shows the same trade-off in modern ML: replay buffers are the hippocampus, the slow-changing parameters are cortex.

**Already in agent systems?** Partially. Letta's core/archival split is *organizational* (context vs. disk) not *computational* (fast vs. slow learners). Mem0 only has a single learner. Mastra OM gets closer with Observer→Reflector (fast → slow compression).

**Ditto proposal.** Two physically separated stores with explicit interfaces:
- **HC (hippocampus):** episodic event records, sparse, write-on-every-event, decays unless replayed.
- **NC (neocortex):** semantic facts/schemas, written only via *consolidation passes* that read HC.
- Reads first check NC (cheap), fall back to HC, and *combine* them (a fact + the episode where it was learned).

### 2.2 Hippocampal Indexing Theory

**Biology.** Teyler & DiScenna (1986, *Behav Neurosci*); Teyler & Rudy (2007, *Hippocampus*) — the hippocampus does not contain the experience, only an *index* into the cortical patterns that constitute the experience. Optogenetic engram reactivation (Liu, Ramirez & Tonegawa 2012, *Nature*) is the cleanest evidence: stimulating a hippocampal index reinstates a cortical-amygdalar pattern.

**Already in agent systems?** Hindsight is closest — it has graph nodes that point at memories; the graph is the index. MemPalace's "drawers" are actually the *content*; the loci are addresses, not pointers. Vector stores treat the index and content as the same object.

**Ditto proposal.** Episodic memory should store *pointers + sparse keys*, not content. The actual content (transcripts, tool outputs, files touched) lives in a content-addressable blob store (think: git-style hash). An episode = `{sparse_key, [content_hash, ...], context}`. This makes episodic storage 100x cheaper and matches the biology: the hippocampus is small (~1% of cortex by volume) precisely because it indexes rather than stores.

### 2.3 Sharp-wave ripples and offline replay

**Biology.** Wilson & McNaughton (1994) showed sleep replay; Karlsson & Frank (2009, *Nat Neurosci*) showed *awake* replay of remote experience; Joo & Frank (2018, *Nat Rev Neurosci*) is the canonical review. Critically, Yu, Liu, Frank et al. (2024, *Science*, "Selection of experience for memory by hippocampal sharp wave ripples") showed that awake ripples *select* which experiences will be consolidated later — disrupting awake ripples impairs subsequent sleep consolidation of those specific experiences. Singer & Frank (2009) showed ripple replay is enhanced near rewards.

**Implication.** Replay isn't background dreaming — it's the brain's commit log. And it's *selective*: high-reward/high-novelty events get replayed disproportionately.

**Already in agent systems?** GBrain has an explicit "Dream Cycle". Mastra OM consolidates on token pressure. Mem0 extracts facts at write time (no replay). None implement *prioritized* replay.

**Ditto proposal.** Two replay processes:
- **Awake ripple** (synchronous, between agent turns, <100ms budget): replay the last N episodes weighted by `surprise * reward * recency`, tag the winners.
- **Dream cycle** (asynchronous, at session end or scheduled): for each tagged episode, attempt schema-fit; if it fits, write a semantic fact to NC and weaken the HC trace; if it doesn't fit, leave the HC trace strong and accumulate evidence.

### 2.4 Schema theory

**Biology.** Bartlett (1932, *Remembering*) — "War of the Ghosts" study. Tse, Langston, Kakeyama et al. (2007, *Science*, "Schemas and memory consolidation") — rats with a pre-existing flavor-place schema consolidated new pairings in 24 hours vs. weeks for naive rats. The schema lives in mPFC; the hippocampus still encodes, but cortex grabs the new fact within a day if it slots into existing structure. van Kesteren et al. (2012, *TiNS*) "SLIMM" model formalized schema-gated routing.

**Implication.** New evidence is not equal evidence. Fits → consolidate fast. Conflicts → keep in episodic, accumulate, possibly trigger schema revision.

**Already in agent systems?** No system I'm aware of explicitly gates consolidation by schema fit. Mem0's update rules check for *contradiction* but treat "consistent new fact" and "novel new fact" identically.

**Ditto proposal.** At consolidation time, compute schema-fit score: does this fact reduce or increase the entropy of the relevant NC subgraph? If reduces (confirms schema): fast-write to NC, optionally weaken HC. If increases (novel/conflicting): defer; require multiple corroborating episodes before NC commit; flag schema for revision.

### 2.5 Reconsolidation

**Biology.** Nader, Schafe & LeDoux (2000, *Nature*) — reactivated fear memories in rat amygdala became labile and required protein synthesis to re-stabilize; blocking it erased the memory. Lee et al. (2017) extended to declarative memory. The labile window is ~6 hours. Hupbach et al. (2007) demonstrated reconsolidation-mediated *updating*: information presented during the labile window is integrated into the original memory.

**Implication.** Retrieval is the perfect moment to update. The agent's act of looking something up should expose that memory to corrections from current context.

**Already in agent systems?** Mem0 has update/contradict logic. Letta exposes write tools the agent can call on recall. Zep can supersede facts via its temporal graph. *None* implement a bounded "labile window" — after a retrieval, accept low-friction edits for the next N minutes/turns, then re-stabilize.

**Ditto proposal.** Every retrieval opens a labile window on the retrieved trace. During the window, any contradiction or elaboration in the agent's subsequent reasoning is captured as a candidate edit. At window close, integrate edits and re-write. This is essentially a high-frequency, low-friction continual update loop that none of the current systems do well.

### 2.6 Memory traces / engrams

**Biology.** Tonegawa lab — Liu et al. (2012, *Nature*) tagged DG engram cells with channelrhodopsin during fear conditioning; optical stimulation later was sufficient to evoke freezing in a neutral context. Ramirez et al. (2013) created false memories by stimulating an engram in a different context. Roy et al. (2016) showed engrams *exist* even in retrograde amnesia — the access path is broken, not the trace. Distributed engrams (Roy et al. 2022, *Nat Commun*) — a single memory is held across multiple regions as a *complex*.

**Implication.** Memories are sparse, content-addressable, and distributed across stores. The same memory has multiple co-active components.

**Already in agent systems?** Hindsight's multi-modal retrieval (vector + BM25 + graph + temporal, fused via RRF) approximates the multi-component engram. MemPalace's spatial hierarchy is the opposite of sparse-distributed.

**Ditto proposal.** Every episodic record is encoded with multiple keys (sparse semantic key, temporal key, entity-graph key, surprise scalar). Retrieval requires *partial* match on any subset (pattern completion). This is essentially Hindsight's design plus an explicit sparsity constraint at write time.

### 2.7 Predictive coding

**Biology.** Rao & Ballard (1999, *Nat Neurosci*) — hierarchical visual cortex as a prediction error minimizer. Friston (2010, *Nat Rev Neurosci*) generalized as the free-energy principle. Memory-relevant: Henson & Gagnepain (2010, *Hippocampus*) — hippocampus signals prediction errors, gates encoding. Greve et al. (2017, *Cortex*) — surprise/violation drives one-shot encoding.

**Implication.** Salience = prediction error. Don't store what you could have predicted.

**Already in agent systems?** None explicitly. Mem0 extracts facts the LLM finds "salient" — but this is the model's qualitative judgment, not a computed prediction error.

**Ditto proposal.** Before writing an episode, ask the NC store: "given context, what did you predict would happen?" Compare to what did happen. Store the residual. If the residual is small (boring, predictable), drop it. If it is large (surprising), store with high salience tag. This is cheap: it's one extra LLM call against the existing NC summary.

### 2.8 Tolman-Eichenbaum Machine, place/grid/time cells

**Biology.** Whittington, Muller, Mark, Behrens et al. (2020, *Cell*) — TEM factorizes structure (entorhinal grid-like codes) from content (lateral entorhinal sensory codes) and binds them in hippocampus, generalizing across environments. Behrens et al. (2018, *Neuron*) — same cognitive map for non-spatial knowledge. Stachenfeld, Botvinick & Gershman (2017, *Nat Neurosci*) — place cells implement successor representations. Time cells (Eichenbaum 2014) — sequence position cells in CA1.

**Implication.** The brain stores knowledge as a *factored graph* — structural roles separate from content. A new domain reuses the same structure.

**Already in agent systems?** Zep's temporal knowledge graph and Hindsight's graph traversal are the closest. Both treat the graph as content-only; neither factors out reusable structural templates.

**Ditto proposal.** NC semantic store is a typed property graph: nodes have types (Person, Project, Bug, Decision, etc.), edges have relational types (CausedBy, BlockedBy, MentionedIn). Templates are first-class: a "Bug" schema specifies expected edges. When a new domain arrives, the agent can *transfer* schema templates (TEM-style generalization). Successor-representation queries — "what tends to follow this state?" — become primitive operations.

### 2.9 Tulving episodic/semantic + MTT + Trace Transformation

**Biology.** Tulving (1972) introduced the distinction. Squire's standard consolidation model says episodes become semantic over time (hippocampus → cortex). Nadel & Moscovitch (1997, *Curr Opin Neurobiol*) — Multiple Trace Theory — episodes *always* require hippocampus; only their gist becomes hippocampus-independent. Winocur & Moscovitch (2011), Sekeres et al. (2018) — Trace Transformation Theory: with time, the trace doesn't move, it *transforms* — episodic detail decays, gist persists. Modern evidence (Bonnici et al. 2012) supports trace transformation.

**Implication.** Don't model consolidation as "move episode to semantic store." Model it as "extract gist, retain or decay episode separately." The episode and the gist coexist for a while; eventually the episode fades but the gist persists.

**Already in agent systems?** GBrain has "compiled truth + timeline" which is roughly this. Mastra OM's Reflections-over-Observations is also close.

**Ditto proposal.** Every episode generates one or more semantic claims at consolidation. The episode persists in HC with decay; the claim persists in NC with reinforcement on each corroborating episode. *Both* are queryable. Crucially: don't delete episodes prematurely — they support trace transformation, future replay, and reconsolidation.

### 2.10 Episodic future thinking

**Biology.** Schacter, Addis & Buckner (2007, *Nat Rev Neurosci*) — same hippocampal-prefrontal-parietal network supports remembering and imagining. Hassabis & Maguire (2007) — hippocampal damage impairs scene construction for *future* events. Suddendorf & Corballis (2007, *BBS*) — mental time travel.

**Implication.** Memory is a *generative* substrate. The same store that returns "what happened" should support "what might happen if…"

**Already in agent systems?** Essentially none. Letta's archival could be queried hypothetically, but no system is designed around recombination.

**Ditto proposal.** Expose a `simulate(query, hypothetical_context)` primitive that retrieves episodic fragments and asks the LLM to recombine them into a plausible future scenario. This makes memory directly useful for planning, not just lookback. Pair with the engram view (§2.6): hypotheticals are pattern completion from *partial novel cues*.

### 2.11 Forgetting as a feature

**Biology.** Anderson, Bjork & Bjork (1994, *JEP:LMC*) — retrieval-induced forgetting: retrieving A from category C inhibits unretrieved B in C. Wimber et al. (2015) — neural evidence in fMRI. Shereshevsky (Luria 1968, *Mind of a Mnemonist*) — couldn't forget, couldn't abstract, couldn't function socially. Hardt, Nader & Nadel (2013, *TiCS*) — active forgetting via AMPA receptor endocytosis.

**Implication.** Without forgetting: interference, slow retrieval, no abstraction, no priorities.

**Already in agent systems?** Mem0 prunes on contradiction. Letta has archival eviction. None implement *active* retrieval-induced suppression. None impose an information budget.

**Ditto proposal.** Three explicit forgetting mechanisms:
- **Decay**: HC traces lose salience monotonically unless replayed.
- **Retrieval-induced suppression**: when episode A is retrieved for query Q, near-neighbor episodes that *would* have matched Q are penalized (this prevents future interference and accelerates retrieval for the canonical exemplar).
- **Hard budget**: total NC node count capped; weakest nodes pruned on insert.

### 2.12 Working memory

**Biology.** Baddeley (1974, 2000) — central executive, phonological loop, visuospatial sketchpad, episodic buffer. Cowan (2005) — embedded processes: focus of attention (~4 items) inside activated long-term memory. Oberauer (2002) — three-state model: direct access region + focus.

**Implication.** "The context window" is not working memory. Working memory has *structure* — a small attentional focus, a slightly larger activated set, and a controller that swaps between them. The LLM's full input is closer to Cowan's "activated long-term memory" than to the focus.

**Already in agent systems?** Letta is closest — its core memory is essentially structured working memory blocks (persona, human, etc.), and the LLM can edit them. Mastra's separation of message history, observations, reflections is also structured.

**Ditto proposal.** Working memory is *not* the context window. It is a small set of typed slots (current goal, current sub-goal, current hypothesis, recent observations, active entities, scratchpad) maintained as structured state outside the LLM input and rendered into the prompt deterministically. This matches Baddeley's component view and makes WM inspectable and editable.

### 2.13 Hebbian plasticity and BCM

**Biology.** Hebb (1949) — "fire together, wire together." Bienenstock, Cooper & Munro (1982) — BCM rule adds a sliding threshold for LTP/LTD, fixing Hebb's stability problem. Modern: Bricken et al. (2023) — sparse distributed networks with Hebbian updates are continual learners; Journé et al. (2023, *Nat Neurosci*) — Hebbian + predictive plasticity learns invariant object representations.

**Implication.** Co-activation should strengthen associative links cheaply, without gradient descent.

**Already in agent systems?** Zep and Hindsight build graph edges between co-mentioned entities — this is essentially Hebbian. None have BCM-style normalization (links saturate without ceiling).

**Ditto proposal.** NC graph edges have a strength scalar updated by a BCM-like rule: co-retrieval in the same query strengthens the edge; the strengthening is divisively normalized by total outgoing edge weight per node (prevents popular nodes from dominating). This makes the graph a learned associative memory.

### 2.14 Sparse Distributed Memory (SDM)

**Biology / theory.** Kanerva (1988) — store/retrieve high-dim binary vectors via Hamming-ball voting. Properties: graceful degradation, content-addressability, associative completion. Bricken et al. (2023, ICLR, "Sparse Distributed Memory is a Continual Learner") — a modern MLP that is mathematically SDM avoids catastrophic forgetting.

**Strengths/weaknesses.** Strengths: scale-free associative recall, robustness, biologically plausible. Weaknesses: binary high-dim representation is awkward for LLMs that emit dense embeddings; doesn't natively support structured queries.

**Already in agent systems?** No major agent memory uses SDM in production. Vector DBs are dense not sparse.

**Ditto proposal.** Use SDM as an *associative side-channel* — not the primary store. A sparse high-dim hash of every NC node enables fast pattern-completion retrieval ("here is half a memory, return the rest") that complements dense embedding search.

### 2.15 Memory consolidation timing

**Biology.** Squire-style: weeks-to-years for systems consolidation in humans (Squire & Alvarez 1995). Tse et al. (2007): 24h with pre-existing schema. Modern: Yang et al. (2014, *Science*) — sleep grows persistent dendritic spines within hours. Born & Wilhelm (2012) — overnight is sufficient for many declarative tasks.

**Implication.** Consolidation cadence depends on schema availability. A new domain warrants long buffering; a familiar one warrants fast commit.

**Ditto proposal.** Adaptive consolidation cadence:
- Per-turn ripples (fast, salience-gated, prep candidate facts).
- Per-session dream cycle (≤ a few minutes after task end): commit schema-fit facts, defer novel ones.
- Per-day deep consolidation: schema revision, graph cleanup, retrieval-induced suppression sweep, decay tick.
- Per-week / "long sleep": global pass — find stale conflicts, archive cold subgraphs, write summary docs.

### 2.16 Sleep-dependent consolidation, SWS vs REM, TMR

**Biology.** Stickgold & Walker (2005, *Nature*; 2013) — SWS for declarative, REM for procedural/emotional. Diekelmann & Born (2010) — active systems consolidation: SWS hippocampal replay coupled to thalamocortical spindles drives cortical strengthening. Rasch et al. (2007, *Science*) — TMR: odor cues during SWS reactivate cued memories, improving recall. Schreiner & Rasch (2015) — TMR strengthens *weakly* encoded items disproportionately.

**Implication.** Two consolidation modes with different goals. And: external cues can bias what gets consolidated.

**Ditto proposal.** Map cleanly:
- **SWS analogue**: replay episode → strengthen NC graph edges, write semantic claims (declarative).
- **REM analogue**: recombine fragments → schema revision, abstraction, "creative" pass (don't bind to specific episodes).
- **TMR analogue**: the user/harness can *cue* the dream cycle — "spend tonight thinking about the auth refactor." This biases which episodes are replayed.

### 2.17 Cognitive offloading and external memory

**Biology / psych.** Sparrow, Liu & Wegner (2011, *Science*) — Google effect: we remember *where* not *what*. Risko & Gilbert (2016, *TiCS*) — cognitive offloading review. Wegner (1986) — transactive memory.

**Implication.** Humans don't try to store everything internally. They store pointers to external systems. Filesystems, wikis, codebases, ticket trackers are extensions of the agent's memory.

**Already in agent systems?** Letta has filesystem awareness. MemPalace stores verbatim, which is the *opposite* of offloading.

**Ditto proposal.** NC should preferentially store *pointers* to authoritative external sources (git commit SHA, file path, ticket ID, URL) plus a small gist, rather than re-encoding the content. When the content is needed, fetch it. This dramatically reduces storage and (importantly) keeps the memory in sync with the world (which agent memory systems are notoriously bad at).

### 2.18 Schema-driven encoding errors / false memories

**Biology / psych.** Bartlett's "War of the Ghosts" (1932) — schema-conformant distortions. Loftus & Palmer (1974) — "smashed" vs "hit" changed reported speed and inserted false broken glass. Roediger & McDermott (1995, *JEP:LMC*) — DRM lists produce reliable false recall of the gist word.

**Implication.** Any memory system that abstracts to gist *will* hallucinate schema-consistent details. This is a feature for compression and a liability for faithfulness.

**Already in agent systems?** This is *exactly* the failure mode of Mem0-style extraction. MemPalace explicitly avoids it (verbatim storage) at the cost of compression.

**Ditto proposal.** Two stores again: NC (gist, will distort) + HC (episodic, verbatim or near-verbatim with content hashes pointing to external). When the agent commits to a recalled fact, surface its provenance: "I recall X (NC gist, confidence 0.7) from episodes [e1, e2] — want me to pull the verbatim?" The HC pointers serve as a falsifiability check. This is how scientific notebooks work and how brains *should* work.

### 2.19 Spacing effect, retrieval practice, testing effect

**Biology / psych.** Ebbinghaus (1885) — spaced study beats massed. Roediger & Karpicke (2006, *Psych Sci*) — testing > restudy by 50% at one week. Cepeda et al. (2008) — optimal spacing interval scales with retention interval.

**Implication.** Active retrieval, not passive presence, builds durable memory. The brain re-uses the same trace and strengthens it through retrieval, not through cramming.

**Already in agent systems?** Spaced repetition exists in human-facing tools (Anki, SuperMemo). I have not seen a *self-rehearsing* agent.

**Ditto proposal.** During dream cycles, the agent self-quizzes: "what do I claim about X?" If it recalls correctly from NC, strengthen that claim; if it can't recall, look it up in HC and re-encode. This is retrieval practice. Schedule the rehearsals on an expanding interval (1d, 3d, 7d, 21d) for any fact tagged as load-bearing for the user.

### 2.20 Salience networks and attentional gating

**Biology.** Posner & Petersen (1990; 2012, *Annu Rev Neurosci*) — alerting, orienting, executive attention networks. Locus coeruleus (LC) norepinephrine system gates cortical encoding (Mather, Clewett, Sakaki & Harley 2016, *BBS*) — under arousal, NE boosts high-priority representations and suppresses lower-priority ones ("hotspots"). Phasic LC bursts on salient/surprising stimuli (Aston-Jones & Cohen 2005).

**Implication.** Salience isn't a content property; it's a *gain modulator*. High-salience moments cause sharper, longer-lasting encoding *and* simultaneous suppression of distractors.

**Already in agent systems?** No system has a salience scalar that modulates both encoding strength and inter-item competition.

**Ditto proposal.** Every episode gets a salience scalar at write time = `surprise + user_signal + task_outcome`. Salience controls (a) write strength, (b) replay priority, (c) competitive suppression of co-encoded distractors in the dream cycle.

---

## 3. Where biology and engineering disagree — what to ignore

Biological plausibility is not an objective. Ditto is not a brain; it's running on Postgres + an LLM, not on spiking neurons + ATP. Here's where copying biology would actively hurt.

1. **Don't copy slow consolidation timescales for fast-moving domains.** The brain takes weeks because protein synthesis is slow. Postgres takes milliseconds. The *adaptive* version is schema-fit gating; the *literal* version (wait two weeks before semanticizing) is silly.
2. **Don't copy distortion for its own sake.** Brains hallucinate schema-consistent details (DRM). For a coding agent, that is a bug, not a feature. Always retain HC pointers to verbatim source so the agent can falsify its own gist. Bias *toward* offloading verbatim to external systems (§2.17).
3. **Don't copy capacity limits of working memory.** Cowan says ~4 items. LLMs handle 100k–1M tokens. Use the full window for *activated long-term memory* (Cowan's outer ring) and reserve a small structured zone for the "focus" — but don't artificially limit the activated zone.
4. **Don't copy synaptic plasticity rules literally.** Hebbian/BCM are useful as inspiration for graph-edge updates, but full backprop and modern embedding training are vastly more sample-efficient. Use Hebb for cheap online graph updates, not for representation learning.
5. **Don't copy stochastic forgetting curves.** Biological forgetting is partly metabolic. Ditto can implement *deterministic* importance-weighted eviction. Use Ebbinghaus-shaped decay as a *prior* for salience, not as a literal mechanism.
6. **Don't copy sleep's offline-only nature.** Biology's consolidation must be offline because the same neurons run perception. Ditto can interleave consolidation with action. Run a "micro-ripple" every turn.
7. **Don't copy reconsolidation lability fully.** In rats, retrieval makes memory erasable for hours. For an agent, this is a vulnerability surface (prompt injection during retrieval). Restrict labile windows to corrections coming from authoritative sources (user, tools, verified facts), not from the LLM's own continuation.
8. **Don't pretend grid cells are necessary.** TEM's factorization is a useful design pattern (separate structure from content). The actual hexagonal grid-cell geometry is not — it's a wiring solution to a 2D problem the agent doesn't face.

---

## 4. A concrete biologically-inspired memory architecture for Ditto

A slot-by-slot sketch. Names are biological for clarity; implementations are mundane.

### 4.1 Stores

| Slot | Bio analog | Implementation | Contents |
|---|---|---|---|
| **WM-focus** | Baddeley focus / Cowan focus | In-prompt structured block, ≤ 2k tokens | Current goal, sub-goal, hypothesis, last K observations |
| **WM-activated** | Cowan activated LTM | Rest of context window | Retrieved episodes & facts for this turn |
| **HC-episodic** | Hippocampus + index | Append-only event log + sparse indices (vector + entity + temporal + salience) | `{id, t, sparse_key, salience, content_hash[], context, replay_count, last_retrieved}` — pointers only |
| **HC-blob** | Cortical content patterns | Content-addressed blob store | Verbatim transcripts, tool outputs, file diffs |
| **NC-graph** | Neocortical schemas | Typed property graph (Postgres + pgvector) | Entities, relations, claims, edge strengths, schema templates |
| **NC-doc** | Cortical "compiled truth" | Markdown pages per entity (GBrain-style) | Compiled summary per entity, regenerated by dream cycle |
| **SDM-assoc** | Pattern completion | Sparse high-dim hash table | Optional associative side-channel for fragment-cued recall |

### 4.2 Write path (encoding)

1. **Predict.** Before recording, ask NC: "given context, what did we expect?" Compute residual against actual event.
2. **Salience score.** `salience = w1*surprise + w2*reward_signal + w3*explicit_user_flag + w4*outcome_delta`. Default low.
3. **Always-on HC write.** Append minimal HC-episodic record with pointer to blob. Cost is small; this is the index.
4. **Salience gate.** If salience > τ, flag for next ripple. If < τ, the record exists but will likely decay without replay.
5. **No immediate NC write.** Resist the Mem0 temptation to extract facts at write time. Defer to consolidation.

### 4.3 Read path (retrieval)

1. Build a query envelope from current WM-focus.
2. Hit NC-graph first (cheap, semantic, gist). Get candidate claims with provenance.
3. For high-stakes queries (defined by config: code edits, irreversible tool calls, user-facing facts), follow provenance to HC-episodic, fetch verbatim blobs.
4. Open a *reconsolidation labile window* on retrieved items (e.g., next 3 turns or 5 minutes): contradictions or elaborations from authoritative sources rewrite the trace.
5. Apply retrieval-induced suppression: near-neighbors of the winner take a small salience hit (decays over hours).
6. Update read counters; this feeds future salience.

### 4.4 Replay processes

- **Awake ripple** (between turns, async, budget ~200ms). Replay top-K HC items by `salience * recency_decay`. Each replay: (a) cheap schema-fit check via NC; (b) if fits, queue for fast consolidation; (c) bump salience for next ripple.
- **Dream cycle** (post-session or scheduled, budget seconds–minutes). For each replay-tagged episode:
  - Schema-fit path: extract claim(s), upsert into NC-graph with BCM-like edge updates, regenerate affected NC-doc pages, weaken HC trace.
  - Novel path: keep in HC at high salience; if ≥ N corroborating episodes accumulate, propose new schema (REM-style abstraction); flag for user review on big schema changes.
- **Long sleep** (daily/weekly). Decay sweep on HC; retrieval-induced suppression sweep on NC near-neighbors; cold-subgraph archival; conflict detection across NC claims; self-testing (spaced retrieval) on high-importance claims.
- **TMR cue.** Harness/user can set a focus topic for the next dream cycle ("think about auth refactor"). This biases replay sampling toward matching episodes.

### 4.5 Generative-future interface

A `simulate(prompt, hypothetical_context)` primitive that:
1. Retrieves episodes matching the hypothetical (pattern completion via SDM-assoc).
2. Recombines fragments via the LLM, conditioned on NC schemas (so the simulation respects learned regularities).
3. Returns a candidate future scenario *plus* the episodic fragments it used (provenance).
4. The scenario is *not* written back to HC unless the user/agent confirms it materialized — but it can be written to a separate "imagined" store for later post-mortem.

### 4.6 Invariants the architecture upholds

- HC is cheap (indices + pointers). NC is expensive (compiled truth). Don't confuse them.
- Nothing is consolidated to NC without passing schema-fit or corroboration.
- Every NC claim has provenance to ≥1 HC episode (falsifiability).
- Every retrieval is a write opportunity (reconsolidation), but only from trusted sources.
- Forgetting is explicit, budgeted, and competitive.
- Salience is computed (surprise + reward), not asserted.
- External systems (git, files, tickets) are first-class memory; we store pointers.

### 4.7 Where this differs from each peer system

| System | Key difference from this sketch |
|---|---|
| **Mem0** | Mem0 extracts at write time; this sketch defers to replay. Mem0 lacks the HC/NC split, schema-fit gating, salience scoring, episodic provenance, retrieval-induced suppression. |
| **Zep** | Zep has temporal graph (good NC analogue) but no separate episodic+blob layer, no replay/dream cycle, no schema-fit gate, no reconsolidation window. |
| **Letta** | Letta's core/archival is closest in spirit but is OS-inspired not CLS-inspired — no consolidation loop, no schema gate, no replay, no salience. |
| **Mastra OM** | Observer→Reflector ≈ awake-ripple→dream-cycle, but consolidation is token-budget-triggered not salience-triggered. No HC pointer store, no schema-fit gate, no reconsolidation. |
| **MemPalace** | Verbatim everything; no NC, no abstraction, no forgetting — biologically the opposite of what brains do (and Shereshevsky shows why). |
| **GBrain** | Compiled-truth + timeline is close to NC-doc + HC. Has a "Dream Cycle" — closest existing analogue. Missing: salience-gated selective replay, reconsolidation window, predictive-coding gate, schema-fit fast path, sparse engram indexing. |
| **Hindsight** | Multi-channel retrieval (vector + BM25 + graph + temporal) matches the distributed-engram view well, but no consolidation pipeline, no episodic/semantic split, no replay. |

---

## 5. References (selected)

**Foundational theory**
- Bartlett, F. C. (1932). *Remembering: A Study in Experimental and Social Psychology*. Cambridge.
- Hebb, D. O. (1949). *The Organization of Behavior*. Wiley.
- Tulving, E. (1972). Episodic and semantic memory. In *Organization of Memory*.
- Baddeley, A. D. & Hitch, G. (1974). Working memory. *Psychology of Learning and Motivation*, 8.
- Teyler, T. J. & DiScenna, P. (1986). The hippocampal memory indexing theory. *Behavioral Neuroscience*, 100.
- Kanerva, P. (1988). *Sparse Distributed Memory*. MIT Press.
- McClelland, J. L., McNaughton, B. L. & O'Reilly, R. C. (1995). Why there are complementary learning systems in the hippocampus and neocortex. *Psychological Review*, 102(3).
- Roediger, H. L. & McDermott, K. B. (1995). Creating false memories. *JEP:LMC*, 21(4).
- Nadel, L. & Moscovitch, M. (1997). Memory consolidation, retrograde amnesia and the hippocampal complex. *Curr. Opin. Neurobiol.*, 7.
- Rao, R. P. N. & Ballard, D. H. (1999). Predictive coding in the visual cortex. *Nature Neuroscience*, 2.
- Nader, K., Schafe, G. E. & LeDoux, J. E. (2000). Fear memories require protein synthesis in the amygdala for reconsolidation after retrieval. *Nature*, 406.

**Modern landmarks**
- Anderson, M. C., Bjork, R. A. & Bjork, E. L. (1994). Remembering can cause forgetting: retrieval dynamics in long-term memory. *JEP:LMC*, 20.
- Roediger, H. L. & Karpicke, J. D. (2006). The power of testing memory. *Psychological Science*.
- Tse, D., Langston, R. F., Kakeyama, M., et al. (2007). Schemas and memory consolidation. *Science*, 316.
- Teyler, T. J. & Rudy, J. W. (2007). The hippocampal indexing theory and episodic memory: updating the index. *Hippocampus*, 17.
- Schacter, D. L., Addis, D. R. & Buckner, R. L. (2007). Remembering the past to imagine the future. *Nat. Rev. Neurosci.*, 8.
- Karlsson, M. P. & Frank, L. M. (2009). Awake replay of remote experiences. *Nat. Neurosci.*, 12.
- Friston, K. (2010). The free-energy principle. *Nat. Rev. Neurosci.*, 11.
- Diekelmann, S. & Born, J. (2010). The memory function of sleep. *Nat. Rev. Neurosci.*, 11.
- Liu, X., Ramirez, S., Pang, P. T., et al. (2012). Optogenetic stimulation of a hippocampal engram activates fear memory recall. *Nature*, 484.
- van Kesteren, M. T. R., Ruiter, D. J., Fernández, G. & Henson, R. N. (2012). How schema and novelty augment memory formation. *Trends Neurosci.*, 35.
- Stickgold, R. & Walker, M. P. (2013). Sleep-dependent memory triage. *Nat. Neurosci.*, 16.
- Hardt, O., Nader, K. & Nadel, L. (2013). Decay happens: the role of active forgetting in memory. *Trends Cogn. Sci.*, 17.
- Mather, M., Clewett, D., Sakaki, M. & Harley, C. W. (2016). Norepinephrine ignites local hotspots of neuronal excitation. *Behavioral and Brain Sciences*, 39.
- Stachenfeld, K. L., Botvinick, M. M. & Gershman, S. J. (2017). The hippocampus as a predictive map. *Nat. Neurosci.*, 20.
- Joo, H. R. & Frank, L. M. (2018). The hippocampal sharp wave-ripple in memory retrieval for immediate use and consolidation. *Nat. Rev. Neurosci.*, 19.
- Whittington, J. C. R., Muller, T. H., Mark, S., et al. (2020). The Tolman-Eichenbaum Machine. *Cell*, 183.
- Roy, D. S., Park, Y.-G., Kim, M. E., et al. (2022). Brain-wide mapping reveals that engrams for a single memory are distributed across multiple brain regions. *Nat. Commun.*, 13.
- Bricken, T., Davies, X., Singh, D., Krotov, D. & Kreiman, G. (2023). Sparse Distributed Memory is a Continual Learner. *ICLR*.
- Yu, J. Y., Liu, D. F., Loback, A., Grossrubatscher, I. & Frank, L. M. (2024). Selection of experience for memory by hippocampal sharp wave ripples. *Science*.
- McClelland, J. L. & colleagues (2024–25). Emergence of complementary learning systems through meta-learning. *CCN*.

**Agent memory systems (2024–2026)**
- Mem0 docs and "State of AI Agent Memory 2026".
- Zep / Graphiti documentation.
- Letta (formerly MemGPT) docs; Packer et al., MemGPT.
- Mastra Research, "Observational Memory: 95% on LongMemEval".
- MemPalace benchmarks and arXiv 2604.21284 "Spatial Metaphors for LLM Memory".
- Hindsight & GBrain documentation; vectorize.io comparative reports.

---

*End of report. ~3,700 words.*
