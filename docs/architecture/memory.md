# Memory architecture

Ditto's memory system is the defensible moat. This document is the commitment. Reasoning, benchmarks, and citations are in [`../research/memory.md`](../research/memory.md).

## Thesis

Ditto is Pareto-better than every incumbent on the axes power users pay for. No single technique gets us there. The combination does:

- **bi-temporal KG** (from Graphiti) → temporal correctness
- **observational consolidation** (from Mastra OM) → LongMemEval-class retrieval at low token cost
- **single-writer harness boundary** → linearizability, no torn writes, no multi-writer corruption
- **content-addressed, Ed25519-signed receipts** → verifiable provenance
- **Postgres + pgvector + tsvector** → crash-consistency for free, RLS multi-tenancy, no schema treadmill
- **skills as first-class typed memory with lifecycle** → procedural memory that doesn't rot
- **eval-as-CI** → regression gates on every PR; benchmarks we ship become the discourse

None of hermes-agent, openclaw, openhuman, mempalace, gbrain, Mem0, Zep, Letta, Hindsight, SuperMemory, or Mastra combine all seven.

## Slots

Five typed slots. The data model does not grow beyond this. New use cases fit existing slots or get rejected.

| Slot | Lifetime | Mutability | Provenance | Notes |
|---|---|---|---|---|
| **Working** | end-of-turn | rw | none | In-context scratchpad. Not durable. Not retrievable. |
| **Episodic** | forever | append-only | content-addressed | Raw events: observations, tool calls, user turns. Immutable. SHA-256 PK. |
| **Semantic** | forever (supersession) | bi-temporal | linked to episodic events that produced it | Distilled facts. `(t_created, t_expired, t_valid, t_invalid)`. Contradiction = invalidate, never delete. |
| **Procedural** | until deprecated | rw | linked to creator | Skills. Lifecycle: active → deprecated → archived. Auto-GC by metabolism rules. |
| **Reflective** | forever (supersession) | bi-temporal | linked to source events + consolidation receipt | Higher-order observations from the dream cycle. Auditable as derived. |

The reflective/semantic split keeps the user-provided record clean from consolidator-generated content. Anyone auditing the system can tell what the user said vs. what Ditto inferred.

## Write path

**The harness is the only writer.** Agents emit events; the harness commits them.

```
agent.emit(Event) ─┐
                   ▼
          harness.receive(Event)
                   │
                   ├── event_id = sha256(canonical_json(event))   # content address
                   ├── prev_id = head of tenant's event chain
                   ├── signature = ed25519_sign(install_key, (event_id, prev_id, ts))
                   ├── BEGIN
                   │   INSERT INTO episodic (event_id, prev_id, tenant_id, payload, signature, ts)
                   │   UPDATE chain_head
                   │   COMMIT     # WAL fsync handles durability
                   ├── return Receipt { event_id, prev_id, signature, ts }
                   ▼
        async consolidator queue
```

Properties:

- **Linearizable** — Postgres advisory lock per `(tenant_id, source_id)` serializes writes within a source.
- **Idempotent** — re-emitting the same event hits the content-addressed PK; no duplicate, returns the existing receipt.
- **Crash-consistent** — Postgres WAL. Power loss mid-write leaves either committed or not, never half.
- **Auditable** — every write produces a signed receipt with `prev_id`, forming a per-tenant hash chain. Tampering is detectable offline by any holder of the install's public key.
- **Replayable** — episodic is append-only and content-addressed. The entire semantic + reflective + procedural state can be regenerated from episodic + consolidator config.

## Read path

Three retrieval modes. Agents pick per query; default is `standard`.

| Mode | Stages | p50 budget | LLM calls (hot path) | When to use |
|---|---|---|---|---|
| `cheap` | BM25 (tsvector) + KG entity exact-match | < 5 ms | 0 | ID lookups, recent-fact recall, deterministic tool inputs |
| `standard` | BM25 + pgvector HNSW → RRF → ColBERT-class rerank top-50 → KG 1-hop expansion | < 50 ms | 0 | Default for agent turns |
| `deep` | standard + LLM query expansion (async prefetch) + cross-encoder rerank + multi-hop KG | ~200 ms | 1 (expansion, async) | Hard multi-hop questions; auto-escalated when standard returns low confidence |

Final ranking: `score = α_recency · recency + α_importance · importance + α_relevance · relevance` (Park et al.). Weights are learned per-tenant offline; defaults are hardcoded.

## Consolidation

Two background cadences. Neither blocks the hot path.

- **Online consolidator** — fires every N=20 episodic events. Single-LLM-call extractor (Mem0-style single-pass) proposes semantic candidates. Each candidate runs the Graphiti contradiction check against existing semantic edges; contradictions invalidate the prior edge via `t_invalid`. All writes are receipt-signed.
- **Dream cycle** — fires at session close and every 24h per tenant. Observer/Reflector pair (Mastra OM pattern): Observer compresses recent episodic into traffic-light-tagged observations; Reflector promotes high-confidence observations into reflective records. ADM-style counterfactual verification runs before commit. Skill metabolism runs here: skills with `last_used > 30d` or `tests_pass < 0.7` are marked deprecated.

Failure modes are handled explicitly:
- Consolidator crash mid-cycle: queue is durable; restart resumes.
- LLM extraction error: episodic write is unaffected; consolidator failure is logged and retried with exponential backoff.
- Contradiction-check ambiguity: surface to a human review queue (per gbrain #1539); never auto-resolve below confidence threshold.

## Storage substrate

**Postgres + pgvector + tsvector. SQLite + sqlite-vec for embedded. No bespoke storage.**

Schema discipline (to avoid gbrain's 30+ migration issues):

- **Additive-only.** Columns are added with defaults; never dropped. Removed columns get a `_deprecated_` prefix; the application stops reading them; the column lives forever.
- **View layer for compat.** All application queries go through views named for the API contract, not the table. Underlying tables can evolve; views absorb the change.
- **Format versioning in the receipt.** Each receipt records the schema version that produced it. Old receipts remain replayable forever; new code can reinterpret them.

Index plan:
- HNSW on every vector column with `tenant_id` as a leading partial-index column for hot tenants.
- GIN on `tsvector` for BM25.
- B-tree on `(tenant_id, source_id, ts)` for time-range scans.

Scaling beyond ~50M vectors/tenant: swap pgvector HNSW for pgvector DiskANN when GA; or partition per-tenant into per-tenant schemas with shared role; or shell out to Milvus/Qdrant via the same retrieval port. The data model does not change.

## Multi-tenancy

Lives at the schema level; see [`multi-tenant.md`](./multi-tenant.md) for the full hierarchy.

- Every memory row carries `tenant_id`. RLS enforces. Application sets `app.tenant_id` per request.
- Every episodic event carries `source_id` (per-Composio-connector, per-MCP-server, per-channel). Retrieval supports source-scoped queries.
- Federation: per-region installs share a key directory; cross-region queries fan out, each install signs its result, federator verifies before merging.
- Regulated tenants get database-per-tenant via config flag. Same code path.

## Integration surface

One Rust core, three transports. All transports go through the same write-path validation and produce the same signed receipts.

- **In-process SDK** — Rust core with Python/TS bindings. Zero serialization overhead. Used by Ditto agents.
- **MCP server** — `ditto memory` exposed as MCP tools (search, write, list_sources, get_receipt, verify_receipt). Lingua franca for Claude Code, Cursor, third-party harnesses.
- **HTTP API + sidecar daemon** — for multi-agent, multi-process deployments where Ditto is not the only consumer. Daemon mediates writes so single-writer invariant holds.

Tools exposed on every transport:

```
memory.write(slot, payload, source_id?, capability_token) → Receipt
memory.search(query, mode=cheap|standard|deep, scope?, k?) → [Record + provenance]
memory.get(event_id) → Record
memory.verify(receipt) → bool
memory.list_sources(tenant_id) → [Source]
memory.consolidate(tenant_id, dry_run=false) → ConsolidationReport
```

## Eval

In-tree, regression-gated on every PR. Failures block merge.

| Suite | Where | Frequency | Floor |
|---|---|---|---|
| LongMemEval-M | `eval/longmemeval/` | every PR | 90% across gpt-4o, gpt-5-mini, opus-4.7, sonnet-4.7 |
| BEAM-1M subset | `eval/beam/` | every PR | 65% |
| BEAM-10M full | `eval/beam/` | nightly | 48% |
| Ditto-Provenance-Bench | `eval/provenance/` | every PR | recall ≥ 0.95 |
| Ditto-Isolation-Bench | `eval/isolation/` | every PR | leak rate = 0 |
| Crash-consistency suite | `eval/crash/` | every PR | 0 lost-acknowledged writes, 0 torn writes, 0 broken hash chains |

Provenance-Bench and Isolation-Bench are Ditto-original. Publishing them is part of the positioning: competitors must either run them (catching up to us) or argue they don't matter (losing on the dimensions enterprise buyers care about).

## What this architecture deliberately does not do

- **No CRDT-style multi-writer.** A multi-writer system can't be linearizable; it gives up exactly the property that prevents mempalace-style corruption.
- **No bespoke vector store.** Every memory startup that built one bled out on durability bugs.
- **No silent semantic deletion.** Contradictions invalidate; users can always see what was once true.
- **No retrieval that returns un-provenanced results.** Every record carries its episodic source set.
- **No skill auto-creation without a quality gate.** Hermes #13265's first flaw. Skills are created with explicit user intent or after a passing test, never by a mechanical timer.

## Open questions

- Multi-agent shared memory: does the single-writer invariant force a daemon hop for every multi-agent shared-memory use case? Re-evaluate at month 6 with real workload data.
- Bi-temporal cost on the hot path: how much can we batch contradiction checks into the dream cycle without retrievable-stale-fact windows growing unacceptable? Need benchmarks against simulated multi-update workloads.
- Receipt key management: rotation, compromise recovery, federation key directory — designed but not yet specified.
- DiskANN-in-pgvector GA timing: drives whether we ship Milvus/Qdrant fallback in v0 or v1.

## Related

- [`../research/memory.md`](../research/memory.md) — full landscape research with citations
- [`./multi-tenant.md`](./multi-tenant.md) — tenancy hierarchy and RLS model
- [`./importer.md`](./importer.md) — how hermes/openclaw state maps into these slots
