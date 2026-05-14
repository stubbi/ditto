# Ditto Agent Memory: 2025-2026 arXiv Frontier Research Pass

**Date:** 2026-05-14
**Scope:** What research published in late 2025 through Q2 2026 points at — that no production memory system (Mem0, Zep/Graphiti, Letta/MemGPT, Mastra OM, MemPalace, gbrain, Hindsight, A-MEM) yet ships.
**Bias:** Opinionated. Citations are specific arXiv IDs where available.

---

## 1. Top 10 ideas the field has but no production memory system ships yet

Ranked by how much they would move SOTA if Ditto were the one to land them in production.

### 1. Hypernetwork-driven, on-the-fly LoRA generation from memory contents (Doc-to-LoRA / SHINE)

Sakana's **Doc-to-LoRA** and **SHINE: Scalable In-Context Hypernetwork** (arXiv 2602.06358) train a hypernetwork that emits a LoRA adapter from a document in a single sub-second forward pass. Reported result: near-perfect needle-in-a-haystack at >4x base context, ~50 MB per "document" instead of the >12 GB KV-cache for a 128K window. **Nobody in the memory ecosystem ships this.** Mem0/Zep/Letta all retrieve text into context. A hypernetwork-as-memory turns "load this user's history" into "swap in a 50 MB adapter at session start." This is the single largest architectural leap available in 2026.

### 2. Test-Time Training as the long-context substitute (TTT-E2E)

End-to-End Test-Time Training for Long Context (arXiv 2512.23675, NVIDIA + collaborators, Jan 2026) reframes long-context as continual learning: a sliding-window Transformer with a dynamic MLP layer that absorbs context via next-token prediction at inference, with constant inference latency regardless of context length, and **2.7x faster than full attention at 128K**. Memory becomes "the weights that got updated by this conversation." No production memory layer treats inference-time weight updates as the storage substrate — they all treat the LLM as frozen and bolt retrieval on top.

### 3. RL-trained memory policies (Memory-R1, Mem-α, AgeMem)

Memory-R1 (arXiv 2508.19828) trains a Memory Manager with PPO/GRPO to decide ADD/UPDATE/DELETE/NOOP, with **only 152 training QA pairs**, and beats Mem0-class baselines on LoCoMo/MSC/LongMemEval. Mem-α (arXiv 2509.25911) generalizes this with a five-operation (store/retrieve/update/summarize/discard) policy. **Mem0's April 2026 algorithm is hand-engineered hierarchical extraction; none of the commercial systems learn their memory policy.** Ditto could ship the first production agent whose memory operations are RL-optimized against measured downstream task reward, not heuristic prompts.

### 4. Sparse-Autoencoder concept memory (typed memory pulled from interpretability)

Qwen-Scope (May 2026), CorrSteer (arXiv 2508.12535), and SAE-RSV (arXiv 2509.23799) show SAE features now correspond to legible concepts at scale, can be selected automatically by correlating with task outcomes, and used to steer generation. Nobody combines SAE features with episodic/semantic memory. The pitch: store retrieved memories alongside a sparse-feature signature; at retrieval, steer the model toward the same concept activations the original encoding produced. This is a typed memory layer extracted from the model's own representation, not embeddings.

### 5. Bi-temporal + valid-time provenance with SCITT-style signed receipts

SCITT (draft-ietf-scitt-architecture-22, expiring April 2026) standardizes append-only COSE-signed merkle-tree receipts for arbitrary supply-chain statements. Zep/Graphiti has bi-temporal facts but no cryptographic provenance; Microsoft has experimental signed agent receipts but no integration with a memory layer. **The combination — every memory write yields a SCITT receipt with valid-time, transaction-time, issuer, and Merkle inclusion proof — is unshipped.** For regulated buyers (financial, legal, health) this is the table-stakes feature competitors don't have.

### 6. Sleep-cycle / dream-time consolidation as a first-class scheduled subsystem

SCM (arXiv 2604.20943), NeuroDream (SSRN 5377250), SleepGate, and "Learning to Forget" (arXiv 2603.14517) all propose explicit NREM/REM-style background passes that compress episodic into semantic, prune low-value, and **generate synthetic replays for further consolidation**. Mem0's batch summarization is the weak analogue; nobody schedules cyclic dream phases with novelty-driven replay. Ditto could be the first to ship this with proper observability (which memories were promoted? what was pruned? what synthetic replay was generated?).

### 7. Episodic-semantic boundary detection by surprise / prediction error

Selective Memory (arXiv 2603.15994) and Adaptive Memory Admission Control (arXiv 2603.04549) propose write-time gates that score events on novelty + prediction error + factual confidence + recency + content prior. Production systems write everything or rely on LLM-judged "is this important." The neural prediction-error signal (encoder-derived novelty, not LLM-judged importance) is unshipped and dramatically more efficient.

### 8. MUVERA + late-interaction rerank as the substrate (not BM25+vector+rerank)

PLAID, MUVERA, ColBERTv2, and Vespa late-interaction now hit <1ms multi-vector retrieval over 100M+ docs (MUVERA+Rerank reportedly **3.33x faster than PLAID with +1.7% mAP**). Production memory systems use single-vector pgvector or HNSW; none ship multi-vector token-level late interaction as the default substrate. ColPali/ColQwen extends this to multimodal. This is mature enough to be the new default for any 2026 memory layer.

### 9. Memory-as-typed-tool with metacognitive abstention (RSCB-MC)

"Learning When to Remember" (arXiv 2604.27283) introduces a risk-sensitive contextual bandit that decides whether to query memory at all, without LLM fine-tuning. The agent learns when its parametric knowledge is enough and skips retrieval. **Every production memory system retrieves on every turn.** This is a 30-50% latency/cost win waiting for someone to ship it cleanly.

### 10. Meta's scalable memory layers (parametric KV memory inside the model)

"Memory Layers at Scale" (arXiv 2412.09764) adds a sparsely-activated trainable key-value lookup that scales to **128B memory parameters with constant FLOPs**, doubles factual QA accuracy, and matches 4x-bigger dense models. This is the path to a Ditto-RAFT'd model with native parametric memory slots. Nobody ships a frontier-quality model with memory layers exposed; if Ditto trains or fine-tunes its own model, this is the architecture.

---

## 2. The frontier on each of the 20 angles

### 2.1 Model editing (MEMIT, ROME, REMEDI)

ROME does rank-one MLP edits; MEMIT scales to thousands of edits. **The 2025-2026 verdict is harsh**: "The Mirage of Model Editing" (arXiv 2502.11177, ACL 2025) shows that model editing benchmarks systematically overstate downstream utility. Catastrophic forgetting kicks in after ~10 edits for ROME/KN; MEMIT survives ~40. NAMET (one-line MEMIT modification with noise injection during memory extraction) extends this but still hits a ceiling. **Verdict for Ditto:** do NOT bet on parametric editing as primary memory. Use it sparingly for "stable global facts the user keeps correcting" (e.g., "my name is X, I prefer Y") and keep an external memory for everything else. The hybrid is the right play.

### 2.2 Continual learning / online learning for agents

The CoLLAs 2025 survey (Wang-ML-Lab/llm-continual-learning-survey, CSUR 2025) and Letta's "Continual Learning in Token Space" (2025) converge on: **don't fine-tune weights in production; learn in token space**. LoRA alone does NOT prevent catastrophic forgetting under continual learning (the "close-enough weights" assumption fails on rugged loss landscapes). The two interesting threads:

- **SuRe** (Surprise-Driven Prioritized Replay) + **FOREVER** (Ebbinghaus-curve replay scheduling) — bring replay buffers into agent memory.
- **Adaptive Minds** — domain-specialized LoRA adapters with semantic routing at inference. This is the cleanest production pattern.

Verdict: weights-as-memory is not ready except via hypernetworks (see angle 14).

### 2.3 Test-time training / inference-time adaptation

**TTT-E2E (arXiv 2512.23675)** is the standout. Also: "Test-Time Training Provably Improves Transformers as In-context Learners" (arXiv 2503.11842), "Test-Time Training Done Right" (arXiv 2505.23884). NVIDIA's January 2026 blog frames it as "Context as Training Data." The line between memory and adaptation is dissolving. **Self-Improving LLM Agents at Test-Time** (arXiv 2510.07841) extends this to agents.

### 2.4 Sparse-autoencoder memory and mechanistic interpretability

Survey: arXiv 2503.05613. Practical: Qwen-Scope (May 2026), CorrSteer (arXiv 2508.12535), SAE-RSV (arXiv 2509.23799), "Use SAEs to Discover Unknown Concepts" (arXiv 2506.23845). Goodfire's Ember API has productized SAE steering. **The unshipped move**: memory entries fingerprinted by SAE feature activations, then at retrieval time use those features as additional steering, not just as embeddings.

### 2.5 Long context vs retrieval (2026 update)

Empirical results (sources: Databricks long-context RAG benchmark, MRCR v2, BABILong leaderboard):
- Below 200K tokens: long context wins for coherent multi-hop reasoning.
- 200K-400K on non-Gemini frontier models: RAG over focused chunks beats naive long-context.
- >400K on non-Gemini: RAG almost always wins.
- **Gemini 3 Deep Think** is the only model that holds quality across the full 1M window.
- Latency/cost: 1M context is **~30-60x slower and ~1250x more expensive per query** than RAG.

The decision rule for 2026: **RAG until 200K, then optionally long-context for Gemini, otherwise always RAG.** Long-context is not a memory replacement; it is an in-session working-memory expansion.

### 2.6 Retrieval-augmented fine-tuning (RAFT, RA-DIT, REPLUG)

RAFT (arXiv 2403.10131), RA-DIT (arXiv 2310.01352), RAG-DDR (arXiv 2410.13509, end-to-end differentiable data rewards). The 2026 reality: **frontier models are already retrieval-aware enough** that the RAFT delta on Claude/GPT-5.5/Gemini 3 is small. RA-DIT and RAG-DDR are interesting for in-house models; for a startup shipping on frontier APIs, this is not a fight worth picking. The one case it matters: if you ship a small (3B-8B) edge model for offline/private deployments, RAFT-fine-tuning is mandatory.

### 2.7 Learned retrievers

The 2026 production stack:
- **Embeddings**: Voyage 3, Cohere Embed v4, BGE-M3, Wholembed v3 (the new multimodal multilingual late-interaction model, March 2026).
- **Rerankers**: Cohere Rerank 3, Voyage Rerank 2, Jina Reranker v2.
- **Late interaction**: ColBERTv2 + PLAID for cost, MUVERA for latency-critical paths.
- **Learned sparse**: SPLADE-v3, SPLATE (arXiv 2404.13950) for sparse late interaction.

The new ECIR 2026 workshop on Late Interaction and Multi-Vector Retrieval (arXiv 2511.00444) consolidates this as a field.

### 2.8 Multi-vector / late-interaction retrieval

MUVERA's Fixed Dimensional Encodings bridge multi-vector to standard MIPS, giving ~0.54 ms query times. ColPali/ColQwen extend late interaction to documents-as-images, which is interesting for memory of screenshots, slide decks, PDFs. **Production trade-off:** storage cost is 10-30x single-vector. For a memory layer it's usually worth it.

### 2.9 Hybrid parametric + retrieval memory

RETRO (DeepMind, 2021) is the canonical reference; "Retro-li" (arXiv 2410.00004v2) shows small-scale viability. Meta's "Memory Layers at Scale" (arXiv 2412.09764) is the cleanest 2025 instantiation. The 2026 frame: **all production memory systems are parametric (LLM weights) + retrieval (external store)** already, but the seam is naive. The interesting work fuses them via cross-attention (RETRO) or KV-key-value memory layers, not via prompt concatenation. This needs custom training, so it's only available to labs that train their own models.

### 2.10 RL-based memory selection

Memory-R1 (arXiv 2508.19828), Mem-α (arXiv 2509.25911), MEM1 (arXiv 2506.15841), Mem-T (arXiv 2601.23014, "Densifying Rewards for Long-Horizon Memory Agents"), MemReward (arXiv 2603.19310). These are the most actionable RL papers for memory. The pattern: tool-use formulation + GRPO + outcome reward + step-level credit assignment. ICLR 2026 has a MemAgents workshop dedicated to this.

### 2.11 Schema / cognitive maps

TEM (Whittington et al. 2020, Cell), TEM-t (transformer variant) — these are predominantly neuroscience. **No serious LLM-memory work has translated TEM into a memory architecture** as of Q2 2026. There is a gap: schema-based memory (Bartlett) is invoked rhetorically (REMI arXiv 2509.06269 mentions "causal schema memory") but no full TEM-style memory layer ships. This is a research bet, not a production move.

### 2.12 Memory compression / token compressors

In-Context Autoencoder ICAE (arXiv 2307.06945, ~4x compression with ~1% extra params), In-Context Former (arXiv 2406.13618), Semantic-Anchor Compression / SAC (arXiv 2510.08907 — "Autoencoding-Free Context Compression via Contextual Semantic Anchors"), CCF (arXiv 2509.09199), ACON (long-horizon LLM agents), Pretraining Context Compressor (ACL 2025). LLMLingua remains the practical baseline. "Goldfish to Elephant" (Cambridge Open Engage) ties selective storage + hierarchical compression. The 2026 winner for production is **SAC** if you can train it; otherwise LLMLingua-2 with structured compression rates per memory type.

### 2.13 Episodic-semantic boundary detection

Position paper: "Episodic Memory is the Missing Piece for Long-Term LLM Agents" (arXiv 2502.06975, Pink et al.). Concrete systems: Selective Memory write-time gating (arXiv 2603.15994), True Memory ingestion gates, A-MAC five-dimensional admission control (arXiv 2603.04549), Continuum Memory Architectures (arXiv 2601.09913). The boundary signal in 2026 work is usually: composite of (semantic novelty vs existing memories) + (encoder prediction error) + (LLM-judged salience). The cheap version: cosine novelty vs the user's last 50 memories. The expensive but better version: actual encoder log-likelihood-based surprise.

### 2.14 Hippocampal indexing systems

Behrens / Whittington / Spalla groups (Oxford, UCL) continue TEM-line work; very little LLM crossover. The most concrete production-relevant idea: **pattern separation** (CA3-like) — write incoming memories with explicit decorrelation against existing memories, so retrieval-by-cue doesn't collapse. Nobody ships this. It's an open opportunity but speculative.

### 2.15 Sleep / dream cycles

The 2026 frontier: SCM (arXiv 2604.20943), NeuroDream (SSRN 5377250), SleepGate (arXiv ID not surfaced, transformer KV-cache sleep gate), "Learning to Forget" (arXiv 2603.14517 — sleep-inspired memory consolidation for proactive interference), "LLM Sleep-Based Learning" (Gallahat, 2026). Open-source: stewnight/rem-sleep-skill, LeoYeAI/openclaw-auto-dream. The interesting variant beyond ADM/SCM: **synthetic dream replay** — generating counterfactual rollouts of recent episodes, scoring them, and using high-value synthetic episodes to update memory or LoRA adapters. This is a step above plain summary consolidation.

### 2.16 Counterfactual / abductive memory

Sparse coverage. REMI (arXiv 2509.06269) — Causal Schema Memory. "Neuro-Symbolic Verification for Preventing LLM Hallucinations in Process Control" (MDPI 2025) frames hallucination as abductive failure and proposes abductive + counter-abductive operators. **No memory system uses counterfactual verification on writes.** This is genuinely open territory: before a write, ask "what would the model have predicted without this memory? Does this memory change downstream answers in ways that are verifiable against ground-truth signals?"

### 2.17 Reflexive / metacognitive memory

"Position: Truly Self-Improving Agents Require Intrinsic Metacognitive Learning" (arXiv 2506.05109, ICML 2025) — the framing paper. "Domain-level Metacognitive Monitoring in Frontier LLMs: A 33-Model Atlas" (arXiv 2605.06673) — empirical. "Learning When to Remember" / RSCB-MC (arXiv 2604.27283). HyperAgents (arXiv 2603.19461). "Hallucinations Undermine Trust; Metacognition is a Way Forward" (arXiv 2605.01428). AbstentionBench shows reasoning-tuned models are often **worse** at knowing when to abstain. This is a real gap.

### 2.18 Provenance and signed receipts (2026 update)

IETF SCITT (draft-ietf-scitt-architecture-22) is the standard. Microsoft Azure has a SCITT transparency service; an in-toto-style profile for AI agents is in early drafts. **VeritasChain (draft-kamimura-scitt-vcp-01)** is a financial-trading audit profile of SCITT — relevant template for an "agent action audit" profile. No memory system ships SCITT-compliant receipts; the closest is Microsoft's experimental agent governance toolkit (signed receipts mentioned in your prior research). The gap: a memory layer where every WRITE/UPDATE/DELETE/READ emits a COSE-signed receipt anchored in a transparency log.

### 2.19 Federated / privacy-preserving memory

Active but generic: PPFL surveys (arXiv 2504.17703), TEE+DP+FL stacks dominate. **No agent-memory-specific FL paper has landed.** The actionable production move: differential-privacy noise on memory exports / sync, and per-user encryption keys with TEE-protected retrieval. This is a compliance feature, not a research bet.

### 2.20 Benchmarks beyond LongMemEval/BEAM

The 2026 wave:
- **LongMemEval-V2** (arXiv 2605.12493) — "Toward Experienced Colleagues" — multi-month workloads.
- **Memora** (arXiv 2604.20006) — "From Recall to Forgetting" — selective forgetting as a first-class metric.
- **MEME** (arXiv 2605.12477) — Multi-entity & Evolving Memory Evaluation.
- **MemoryAgentBench** — accurate retrieval + test-time learning + long-range understanding + selective forgetting.
- **MemBench** — factual vs reflective memory split.
- **Beyond the Context Window** (arXiv 2603.04814) — cost/performance for persistent agents.
- **"Evaluating Long-Term Memory for Long-Context QA"** (arXiv 2510.23730).
- **BABILong** (arXiv 2406.10149) — still the gold standard for long-context reasoning, now extended to 10M tokens.
- **RULER** — synthetic long-context degradation curves.

**Selective forgetting** and **knowledge updates** are the two areas where current benchmarks (LoCoMo, LongMemEval) underestimate the gap between systems.

---

## 3. What Ditto should adopt

Concrete recommendations, ranked by ROI.

### Tier S — adopt now, primary differentiation

1. **RL-trained memory operations policy** (Memory-R1 / Mem-α / AgeMem pattern). 152 QA pairs and GRPO is enough; the policy generalizes. This single feature beats Mem0's hand-tuned heuristics measurably on LoCoMo and LongMemEval. Citations: arXiv 2508.19828, 2509.25911, 2506.15841.

2. **SCITT-signed memory receipts** (every WRITE/UPDATE/DELETE/READ produces a COSE-signed merkle-tree-inclusion-proven receipt). Use draft-ietf-scitt-architecture-22 + a Ditto SCITT profile. This is the regulatory moat competitors cannot easily replicate without a year of work.

3. **Late-interaction multi-vector retrieval as default** (ColBERTv2/PLAID or MUVERA). Reranker layer with Cohere Rerank 3 or equivalent. This is mature and beats BM25+pgvector+RRF on every public benchmark.

4. **Surprise-gated writes**. Encoder prediction-error + cosine novelty composite score; threshold-gate the memory write. Citations: arXiv 2603.15994, 2603.04549, 2601.09913. Cheap, high impact, no competitor ships it.

5. **Metacognitive retrieval gate** (RSCB-MC contextual bandit). The agent learns when it doesn't need memory. Citation: arXiv 2604.27283. 30-50% latency/cost reduction with no quality loss.

### Tier A — adopt next, differentiated

6. **Hypernetwork-based per-user LoRA generation** (Doc-to-LoRA / SHINE). Generate a per-user or per-session LoRA from memory store; load at session start. Citation: arXiv 2602.06358 (SHINE), Sakana's pub.sakana.ai/doc-to-lora. This is the largest architectural bet, but requires running on a model you control (Llama-class, not Claude API).

7. **Sleep-cycle consolidation as a scheduled subsystem** with observability. Three phases: light-sleep ingest/stage, REM reflect/extract, deep-sleep promote/prune. Surface what was promoted, what was pruned, what synthetic replay was generated. Citations: arXiv 2604.20943 (SCM), 2603.14517 ("Learning to Forget"), NeuroDream.

8. **Bi-temporal facts** (valid-time + transaction-time, Graphiti-style) on entity-linked records, but with SCITT receipts. This closes the Graphiti gap and adds verifiability.

9. **Test-time training as optional long-context substitute** when context >200K and the deployment can afford weight updates. Use TTT-E2E (arXiv 2512.23675) where Gemini long-context is not available. This is forward-looking — implement behind a flag.

### Tier B — adopt selectively

10. **Counterfactual write verification** (a Ditto novelty): before persisting a memory, run a small abductive check — does this memory change the model's answer to a held-out probe in a way consistent with the source? If not, downweight or reject the write.

11. **SAE-fingerprinted memories** if Ditto controls the model. Tag each memory with the top-k activated SAE features at encoding time; at retrieval, steer with those features. Citations: Qwen-Scope, arXiv 2509.23799.

12. **Selective forgetting as a first-class API** (write a fact, mark for time-decay, mark for hard delete, mark for export). Memora (arXiv 2604.20006) shows benchmarks now reward this; few systems support it cleanly.

### Tier C — track but defer

- Memory layers at scale (Meta) — only if Ditto trains its own model.
- TEM / hippocampal cognitive maps — research bet, not production-ready.
- Federated learning for memory — wait for a customer demand signal.

---

## 4. What Ditto should NOT adopt (and why)

### Parametric model editing (ROME/MEMIT) as a primary memory mechanism

"Mirage of Model Editing" (arXiv 2502.11177) and the catastrophic-forgetting literature show edits compound destructively. MEMIT survives ~40 edits; production memory needs millions. Use it for at most "stable identity facts the user re-asserts often" — and even there, test against degradation. Not a primary substrate.

### LoRA fine-tuning as the user-memory mechanism (Adaptive Minds style)

LoRA does NOT prevent catastrophic forgetting under continual updates. The exception is hypernetwork-generated LoRAs (you regenerate from scratch each time from the document store) — that's fine. But a per-user growing LoRA stack will degrade.

### RAFT / RA-DIT for frontier-model deployments

If Ditto ships on Claude / GPT-5.5 / Gemini 3, RAFT-style retrieval-aware fine-tuning gives marginal gains because the frontier models are already strong at open-book QA. RAFT is only worth it if Ditto ships a small private/edge model.

### Naive 1M-token long-context as a memory replacement

Empirical 2026 result: only Gemini 3 Deep Think holds quality across 1M, and even there latency is 20-30s for first-token. Cost is ~1250x RAG. Long-context is in-session working memory, not long-term memory.

### Full TEM / cognitive-map architectures

Beautiful research, no production-ready system. Spend the cycles on RL memory policy and hypernetworks instead.

### Generic federated learning

Privacy compliance is real but the right answer for 2026 is per-tenant encryption + TEE retrieval + audit receipts — not federated training of a memory model.

---

## 5. Citations (selected)

### Memory systems and RL-trained memory

- Yu et al., **Memory-R1**: Enhancing LLM Agents to Manage and Utilize Memories via RL. arXiv 2508.19828. https://arxiv.org/abs/2508.19828
- **Mem-α**: Learning Memory Construction via Reinforcement Learning. arXiv 2509.25911.
- **MEM1**: Learning to Synergize Memory and Reasoning for Efficient Long-Horizon Agents. arXiv 2506.15841.
- **Mem-T**: Densifying Rewards for Long-Horizon Memory Agents. arXiv 2601.23014.
- **MemReward**: Graph-Based Experience Memory for LLM Reward Prediction. arXiv 2603.19310.
- Pink et al. **Position: Episodic Memory is the Missing Piece for Long-Term LLM Agents.** arXiv 2502.06975. https://arxiv.org/abs/2502.06975
- **A-MEM**: Agentic Memory for LLM Agents. arXiv 2502.12110.
- **Agentic Memory: Learning Unified Long-Term and Short-Term Memory Management.** arXiv 2601.01885.
- **Mem0**: Building Production-Ready AI Agents with Scalable Long-Term Memory. arXiv 2504.19413.
- **Memory in the Age of AI Agents** (survey). arXiv 2512.13564.
- **From Storage to Experience: A Survey on the Evolution of LLM Agent Memory.** arXiv 2605.06716.
- **Memory for Autonomous LLM Agents: Mechanisms, Evaluation, and Emerging Frontiers.** arXiv 2603.07670.

### Test-time training and hypernetworks

- **End-to-End Test-Time Training for Long Context.** arXiv 2512.23675 (Jan 2026). https://arxiv.org/abs/2512.23675
- **Test-Time Training Provably Improves Transformers as In-context Learners.** arXiv 2503.11842.
- **Test-Time Training Done Right.** arXiv 2505.23884.
- **Self-Improving LLM Agents at Test-Time.** arXiv 2510.07841.
- **Let's (not) just put things in Context: Test-Time Training for Long-Context LLMs.** arXiv 2512.13898.
- **Titans: Learning to Memorize at Test Time.** Behrouz et al. arXiv 2501.00663 (NeurIPS 2025). https://arxiv.org/abs/2501.00663
- **Titans Revisited.** arXiv 2510.09551.
- **SHINE: A Scalable In-Context Hypernetwork for Mapping Context to LoRA in a Single Pass.** arXiv 2602.06358.
- Sakana AI. **Doc-to-LoRA / Text-to-LoRA.** https://pub.sakana.ai/doc-to-lora/

### Model editing

- **MEMIT** — Mass Editing Memory in a Transformer. https://memit.baulab.info/
- **Knowledge Editing for LLMs: A Survey.** ACM Computing Surveys, 2025. doi 10.1145/3698590
- **The Mirage of Model Editing** (ACL 2025). arXiv 2502.11177. https://arxiv.org/abs/2502.11177

### Continual learning

- **Continual Learning of LLMs: A Comprehensive Survey** (CSUR 2025). Wang-ML-Lab repository.
- **Defying Catastrophic Forgetting in Continual LLM Unlearning.** arXiv 2601.21682.
- Letta. **Continual Learning in Token Space.** 2025.

### SAEs and interpretability for memory

- **A Survey on Sparse Autoencoders.** arXiv 2503.05613.
- **Use SAEs to Discover Unknown Concepts, Not to Act on Known Concepts.** arXiv 2506.23845.
- **CorrSteer.** arXiv 2508.12535.
- **SAE-RSV (Refinement of Steering Vector via SAE).** arXiv 2509.23799.
- Qwen-Scope (May 2026).

### Compression and context as memory

- **In-Context Autoencoder (ICAE).** arXiv 2307.06945.
- **In-Context Former.** arXiv 2406.13618.
- **Autoencoding-Free Context Compression via Contextual Semantic Anchors (SAC).** arXiv 2510.08907.
- **CCF: Context Compression Framework.** arXiv 2509.09199.
- **ACON: Optimizing Context Compression for Long-horizon LLM Agents.** OpenReview 2026.
- **From Goldfish to Elephant: Long-Term Memory via Selective Storage and Hierarchical Compression.** Cambridge Open Engage.

### Sleep / consolidation

- **SCM: Sleep-Consolidated Memory.** arXiv 2604.20943.
- **Learning to Forget: Sleep-Inspired Memory Consolidation for Proactive Interference.** arXiv 2603.14517.
- **NeuroDream.** SSRN 5377250.

### Retrieval (late interaction, sparse, multi-vector)

- **ColBERTv2.** arXiv 2112.01488.
- **PLAID.** arXiv 2205.09707.
- **SPLATE: Sparse Late Interaction Retrieval.** arXiv 2404.13950.
- **Efficient Constant-Space Multi-Vector Retrieval.** arXiv 2504.01818.
- **LIR Workshop @ ECIR 2026.** arXiv 2511.00444.

### Memory layers (parametric)

- Berges et al. **Memory Layers at Scale.** arXiv 2412.09764. Meta.

### Write-time gating, salience, novelty

- **Selective Memory: Write-Time Gating with Hierarchical Archiving.** arXiv 2603.15994.
- **Adaptive Memory Admission Control (A-MAC).** arXiv 2603.04549.
- **Continuum Memory Architectures for Long-Horizon LLM Agents.** arXiv 2601.09913.
- **StageMem: Lifecycle-Managed Memory.** arXiv 2604.16774.
- **Storage Is Not Memory: A Retrieval-Centered Architecture for Agent Recall.** arXiv 2605.04897.

### Metacognition and abstention

- **Position: Truly Self-Improving Agents Require Intrinsic Metacognitive Learning.** arXiv 2506.05109 (ICML 2025).
- **Domain-level Metacognitive Monitoring in Frontier LLMs.** arXiv 2605.06673.
- **Hallucinations Undermine Trust; Metacognition is a Way Forward.** arXiv 2605.01428.
- **Learning When to Remember (RSCB-MC).** arXiv 2604.27283.
- **HyperAgents.** arXiv 2603.19461.
- **Agentic Metacognition: Designing a Self-Aware Low-Code System.** arXiv 2509.19783.

### Counterfactual / causal memory

- **REMI: Causal Schema Memory.** arXiv 2509.06269.

### Provenance / SCITT

- IETF. **An Architecture for Trustworthy and Transparent Digital Supply Chains.** draft-ietf-scitt-architecture-22. https://datatracker.ietf.org/doc/draft-ietf-scitt-architecture/
- **VeritasChain Protocol (VCP).** draft-kamimura-scitt-vcp-01.
- https://scitt.io/

### Benchmarks

- **LongMemEval.** arXiv 2410.10813 (ICLR 2025).
- **LongMemEval-V2.** arXiv 2605.12493.
- **BABILong.** arXiv 2406.10149.
- **Memora: From Recall to Forgetting.** arXiv 2604.20006.
- **MEME: Multi-entity & Evolving Memory Evaluation.** arXiv 2605.12477.
- **Beyond the Context Window** (cost-performance of fact-based memory vs long-context). arXiv 2603.04814.
- **Evaluating Long-Term Memory for Long-Context QA.** arXiv 2510.23730.
- **Memory in the LLM Era: Modular Architectures and Strategies.** arXiv 2604.01707.

### Surveys to keep open in tabs

- **Memory for Autonomous LLM Agents** survey. arXiv 2603.07670.
- **Memory in the Age of AI Agents.** arXiv 2512.13564.
- **From Storage to Experience.** arXiv 2605.06716.
- **Externalization in LLM Agents.** arXiv 2604.08224.
- **Multi-Agent Memory from a Computer Architecture Perspective.** arXiv 2603.10062.

---

## Closing opinion

If Ditto ships only one thing that competitors don't: **make it an RL-trained memory policy with SCITT-signed receipts and surprise-gated writes**, sitting on top of a late-interaction retrieval substrate. That single combination beats every named competitor on quality (RL policy + late interaction), latency/cost (metacognitive retrieval gate), and regulated-buyer compliance (signed receipts). The Doc-to-LoRA hypernetwork play is bigger but only available if Ditto controls the base model; if Ditto is API-bound for now, defer it but build the data pipeline that would feed it later. Test-time training and sleep-cycle consolidation are the right second-wave bets, six to twelve months out.
