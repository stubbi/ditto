"""Ditto-Provenance-Bench.

Given a corpus of episodic events and a question, the backend must return
records whose `source_event_ids` include the events that actually contain
the answer, and exclude events that don't.

Scoring per example:
  precision = |returned ∩ must_include| / |returned|       (or 1 if returned ⊆ must_include and not empty)
  recall    = |returned ∩ must_include| / |must_include|
  hit       = no event in `must_not_include` appears in returned source_event_ids
  score     = (recall * hit) — recall, gated to zero if the backend leaks a forbidden source.

Aggregate: mean score across examples. Pass threshold (v0): >= 0.50.
"""

from __future__ import annotations

from typing import Any

import yaml

from ditto_eval.backends.base import MemoryBackend
from ditto_eval.benchmarks.base import Benchmark, BenchmarkResult, ExampleResult
from ditto_eval.types import Event


class ProvenanceBench(Benchmark):
    name = "ditto-provenance-bench"

    async def run(self, backend: MemoryBackend) -> BenchmarkResult:
        with self.fixture.open() as f:
            spec = yaml.safe_load(f)

        examples: list[ExampleResult] = []
        for ex in spec["examples"]:
            tenant_id = f"prov-{ex['id']}"
            await backend.reset(tenant_id)

            # map fixture event ids ('e1', 'e2', ...) to content-addressed event_ids
            local_to_event_id: dict[str, str] = {}
            prev: str | None = None
            for local in ex["setup"]["events"]:
                event = Event(
                    tenant_id=tenant_id,
                    source_id=local.get("source", "default"),
                    payload={"content": local["content"]},
                    timestamp=float(local["timestamp"]),
                    prev_event_id=prev,
                )
                await backend.write(event)
                local_to_event_id[local["id"]] = event.event_id
                prev = event.event_id

            must_include = {local_to_event_id[i] for i in ex["must_include_source_events"]}
            must_not_include = {local_to_event_id[i] for i in ex.get("must_not_include_source_events", [])}

            results = await backend.search(
                query=ex["query"],
                tenant_id=tenant_id,
                k=ex.get("k", 5),
            )
            returned_sources: set[str] = set()
            for r in results:
                returned_sources.update(r.source_event_ids)

            score, details = self._score(must_include, must_not_include, returned_sources)
            examples.append(ExampleResult(
                example_id=ex["id"],
                passed=score >= 0.5,
                score=score,
                details=details,
            ))

            await backend.reset(tenant_id)

        passed = sum(1 for e in examples if e.passed)
        agg = sum(e.score for e in examples) / len(examples) if examples else 0.0
        return BenchmarkResult(
            benchmark=self.name,
            backend=backend.name,
            fixture_version=str(spec.get("version", "0")),
            total=len(examples),
            passed=passed,
            score=agg,
            examples=examples,
        )

    @staticmethod
    def _score(
        must_include: set[str],
        must_not_include: set[str],
        returned: set[str],
    ) -> tuple[float, dict[str, Any]]:
        leak = bool(returned & must_not_include)
        if not must_include:
            return (0.0 if leak else 1.0), {"leak": leak, "no_required": True}
        hit_count = len(must_include & returned)
        recall = hit_count / len(must_include)
        score = 0.0 if leak else recall
        return score, {
            "leak": leak,
            "recall": recall,
            "hits": hit_count,
            "required": len(must_include),
        }
