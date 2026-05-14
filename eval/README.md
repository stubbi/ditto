# ditto-eval

Benchmark harness for agent memory systems. Runs LongMemEval, BEAM, Ditto-Provenance-Bench, and Ditto-Isolation-Bench against pluggable backends (Mem0, Zep, Mastra, MemPalace, gbrain, Ditto, plus a reference stub).

We're starting eval-first on purpose: before Ditto writes a line of memory-engine code, we need credible numbers on the incumbents and a fixed contract for what "winning" means.

## Status

v0.0.1. Implemented:

- `MemoryBackend` protocol (the contract every adapter implements)
- `StubBackend` — reference in-memory substring scanner; control floor
- `DittoBackend` — speaks MCP stdio to a `ditto serve` subprocess
- `Mem0Backend` — gated on `OPENAI_API_KEY` (one bounded competitor adapter)
- `ProvenanceBench` — Ditto-Provenance-Bench runner, with a 3-example v0 fixture
- CLI: `ditto-eval run --benchmark provenance --backend {stub,ditto,mem0}`

## First-eval results

```
ditto-provenance-bench on stub:    0/3  score 0.000   (substring control floor)
ditto-provenance-bench on ditto:   2/3  score 0.667   (RRF + OpenAI embeddings + two-knob relevance gate)
```

Trajectory: BM25-only (0/3) → BM25 + RRF + deterministic embedder (1/3) → + relevance gate on cosine similarity (2/3).

The remaining `prov-002-updated-fact` fail is structural, not a tuning gap:

- prov-001 (passes): "When was the user born?" against a corpus where the right answer ("birthday March 14") has the **highest** cosine similarity. Threshold gate drops distractors.
- prov-002 (fails): "Where does the user currently live?" against a corpus where the **older** record ("lives in SF") has *higher* cosine than the **newer** correct record ("moved to Berlin"). No purely-retrieval threshold can pick the temporally-newer record when its similarity score is lower.
- prov-003 (passes): multi-hop, same shape as prov-001.

These two examples need opposite temporal weights (prov-001 wants the *oldest* event; prov-002 wants the *newest*). A single ranking rule cannot satisfy both. The architecture's per-tenant-learned `α_recency` weight or its bi-temporal NC-graph supersession both solve this — both are deferred per the v0 memory scope. **The current ceiling is 2/3 by design.**

With `OPENROUTER_API_KEY` or `OPENAI_API_KEY` set, the harness wires real semantic embeddings via `OpenAiEmbedder` (OpenRouter routes to `openai/text-embedding-3-small` at the same 1536 dim, no schema change). Auto-selected when the key is in env; override with `DITTO_EMBEDDER=openai|openrouter|deterministic|none`.

Checked-in results under `results/` so the trajectory per backend per date is auditable. We deliberately do **not** publish Mem0/Zep/Mastra comparisons on `LongMemEval` / `BEAM` yet — Ditto's retrieval stack is incomplete, and the post-MemPalace-#214 methodology bar requires matched-conditions BM25 baselines at full corpus scale before any public comparison.

Forthcoming (in roughly this order):

1. `Mem0Backend` adapter (mem0ai SDK)
2. `LongMemEval` runner against the public ICLR 2025 fixture
3. `ZepBackend` adapter
4. `BEAM` runner against the public ICLR 2026 fixture
5. `IsolationBench` — Ditto-Isolation-Bench, adversarial multi-tenant
6. `MemPalaceBackend` and `GBrainBackend` via MCP
7. `MastraBackend` adapter
8. `CrashBench` — process kill mid-write, durability checks
9. `DittoBackend` — once the Rust crate exists

## Install

```bash
cd eval
uv sync
# or: pip install -e .
```

## Run

```bash
ditto-eval list
ditto-eval run --benchmark provenance --backend stub
```

Results are written to `eval/results/<benchmark>_<backend>_<timestamp>.json`. We commit results to the repo so the historical trajectory of each backend is auditable.

## The protocol

A backend is anything that implements `ditto_eval.backends.base.MemoryBackend`:

```python
class MemoryBackend(ABC):
    name: str
    async def write(self, event: Event) -> Receipt: ...
    async def search(self, query, tenant_id, sources=None, k=10, mode="standard") -> list[SearchResult]: ...
    async def verify(self, receipt: Receipt) -> bool: ...
    async def reset(self, tenant_id: str) -> None: ...
```

Five methods. Three core types (`Event`, `Receipt`, `SearchResult`). Backends without multi-tenancy treat `tenant_id` as a namespace. Backends without signing return `None` for `signature` and `True` from `verify`. Backends without provenance tracking return `[]` in `SearchResult.source_event_ids` and score 0 on `ProvenanceBench` (but can still pass LongMemEval/BEAM).

The protocol surface is deliberately minimal — adding a method here means rewriting every adapter. New capabilities go in `metadata` on `SearchResult` first; promoted to first-class only after multiple backends ship them.

## Adding a backend

```python
from ditto_eval.backends.base import MemoryBackend

class MyBackend(MemoryBackend):
    name = "mybackend"
    async def write(self, event):  ...
    async def search(self, query, tenant_id, sources=None, k=10, mode="standard"):  ...
    async def verify(self, receipt):  return True
    async def reset(self, tenant_id):  ...
```

Register in `ditto_eval/cli.py` under `BACKENDS`. Tests in `tests/test_<name>_backend.py`.

## Adding a fixture

Fixtures live under `eval/fixtures/<benchmark>/<version>.yaml`. The format for `ProvenanceBench` is in `fixtures/provenance/v0.yaml`. Fixtures are versioned and never edited in place — bump the version, add `v1.yaml`, leave `v0.yaml` untouched so historical results stay comparable.

## Why eval-first

The architecture doc claims Ditto's memory is Pareto-better than incumbents. That's testable. Building the eval harness before the memory engine forces us to:

1. Measure incumbents on the same fixtures we'll measure ourselves on.
2. Lock down the API contract that the Rust core will have to implement.
3. Have a regression gate ready the day the first `DittoBackend` commit lands.
4. Publish numbers. Ditto-Provenance-Bench and Ditto-Isolation-Bench become public the moment they ship — competitors either run them or argue they don't matter.

See [`../docs/research/memory.md`](../docs/research/memory.md) for the broader landscape and [`../docs/architecture/memory.md`](../docs/architecture/memory.md) for the architecture this is measuring against.
