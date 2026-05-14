"""LoCoMo retrieval-only benchmark.

Measures recall@K directly: for each question, does the gold evidence
dia_id ("D1:3", "D2:8", …) appear in any of the top-K retrieved
snippets? No QA model, no judge — just retrieval quality.

Why this exists alongside the full `LocomoBench`: the full pipeline's
signal (QA accuracy after judging) is dominated by two confounds at
small sample sizes — the QA model's prompt + abstention behavior, and
the judge's leniency. Both are real but they're not what we're tuning
when we swap extractors and rerankers. The retrieval-only signal is
direct, fast (no LLM calls per question), and statistically tighter.

We compute recall@5 and recall@10 per question, plus per-category
aggregates.
"""

from __future__ import annotations

import asyncio
import json
import re
from pathlib import Path

import os

from ditto_eval.backends.base import MemoryBackend
from ditto_eval.benchmarks.base import Benchmark, BenchmarkResult, ExampleResult
from ditto_eval.benchmarks.locomo import (
    CATEGORY_NAMES,
    LocomoConfig,
    _stratify,
    fetch_dataset,
    parse_session_datetime,
)
from ditto_eval.llm import LlmClient
from ditto_eval.query_expansion import expand_query
from ditto_eval.types import Event


class LocomoRetrievalBench(Benchmark):
    """recall@K on LoCoMo evidence dia_ids."""

    name = "locomo-retrieval"

    def __init__(self, fixture: Path, config: LocomoConfig | None = None) -> None:
        super().__init__(fixture)
        self.config = config or LocomoConfig()

    async def run(self, backend: MemoryBackend) -> BenchmarkResult:
        ds_path = fetch_dataset(self.fixture)
        with ds_path.open() as f:
            dataset = json.load(f)
        targets = (
            range(len(dataset))
            if self.config.conversations is None
            else self.config.conversations
        )

        # Query expansion is opt-in via env. We allocate one LlmClient
        # for the bench when enabled — single connection pool, reused
        # across all questions.
        qe_enabled = os.environ.get("LOCOMO_QUERY_EXPANSION", "0").lower() in ("1", "true", "yes")
        qe_llm: LlmClient | None = None
        if qe_enabled:
            qe_llm = LlmClient()

        examples: list[ExampleResult] = []
        by_category: dict[str, list[float]] = {v: [] for v in CATEGORY_NAMES.values()}
        recall5_hits = 0
        recall10_hits = 0
        for conv_idx in targets:
            conv = dataset[conv_idx]
            tenant_id = f"locomo-retrieval-{conv['sample_id']}"
            await backend.reset(tenant_id)

            # Map dia_id -> event_id so we can ground-truth check
            # retrieval against the gold-evidence dia_ids without
            # depending on string equality of content.
            dia_to_event: dict[str, str] = {}
            conversation = conv["conversation"]
            prev: str | None = None
            for skey in sorted(
                [k for k in conversation if re.fullmatch(r"session_\d+", k)],
                key=lambda s: int(s.split("_")[1]),
            ):
                base_ts = parse_session_datetime(conversation.get(f"{skey}_date_time", ""))
                turns = conversation[skey]
                if not isinstance(turns, list):
                    continue
                for i, turn in enumerate(turns):
                    text = turn.get("text", "")
                    if "img_url" in turn or "blip_caption" in turn:
                        cap = turn.get("blip_caption") or turn.get("img_url", "")
                        text = f"{text} [image: {cap}]".strip()
                    if not text:
                        continue
                    event = Event(
                        tenant_id=tenant_id,
                        source_id=turn.get("speaker", "unknown"),
                        payload={"content": text, "dia_id": turn.get("dia_id", "")},
                        timestamp=base_ts + i * 60.0,
                        prev_event_id=prev,
                    )
                    await backend.write(event)
                    if d := turn.get("dia_id"):
                        dia_to_event[d] = event.event_id
                    prev = event.event_id

            # Optional batched dream for LLM-extractor backends.
            if self.config.consolidate_after_ingest:
                await backend.consolidate(tenant_id, mode="dream")

            qa = _stratify(conv["qa"], self.config.max_questions_per_category)
            sem = asyncio.Semaphore(self.config.concurrency)

            async def one(q: dict, conv_id: str = tenant_id) -> ExampleResult:
                async with sem:
                    return await self._one(backend, conv_id, q, dia_to_event, qe_llm)

            conv_results = await asyncio.gather(*[one(q) for q in qa])
            for r in conv_results:
                examples.append(r)
                cat = r.details.get("category", "unknown")
                if cat in by_category:
                    by_category[cat].append(r.score)
                if r.details.get("recall_at_5", 0.0) > 0:
                    recall5_hits += 1
                if r.details.get("recall_at_10", 0.0) > 0:
                    recall10_hits += 1
            await backend.reset(tenant_id)
        if qe_llm is not None:
            await qe_llm.close()

        total = len(examples)
        per_cat = {
            cat: (sum(vs) / len(vs) if vs else None) for cat, vs in by_category.items()
        }
        passed = sum(1 for e in examples if e.passed)
        agg = sum(e.score for e in examples) / total if total else 0.0
        return BenchmarkResult(
            benchmark=self.name,
            backend=backend.name,
            fixture_version="locomo10",
            total=total,
            passed=passed,
            score=agg,
            examples=examples,
            metadata={
                "per_category_recall_at_10": per_cat,
                "recall_at_5": recall5_hits / total if total else 0.0,
                "recall_at_10": recall10_hits / total if total else 0.0,
                "top_k": self.config.top_k,
                "max_questions_per_category": self.config.max_questions_per_category,
            },
        )

    async def _one(
        self,
        backend: MemoryBackend,
        tenant_id: str,
        q: dict,
        dia_to_event: dict[str, str],
        qe_llm: LlmClient | None = None,
    ) -> ExampleResult:
        question = q["question"]
        category = CATEGORY_NAMES.get(q["category"], "unknown")
        evidence_dia_ids = q.get("evidence", [])
        gold_event_ids = {
            dia_to_event[d] for d in evidence_dia_ids if d in dia_to_event
        }
        # Adversarial (cat 5) has no evidence — gold behavior is to NOT
        # retrieve any specific evidence. We skip these for the recall
        # metric.
        if not gold_event_ids:
            return ExampleResult(
                example_id=f"{tenant_id}#adv",
                passed=True,  # no recall expected
                score=1.0,
                details={"category": category, "skipped_no_evidence": True},
            )

        # Query expansion: search each phrasing, union the results,
        # keep best-rank for each event_id across the union.
        if qe_llm is not None:
            variants = await expand_query(qe_llm, question)
        else:
            variants = [question]

        # Per-variant search, merge by lowest rank (best score wins).
        from collections import defaultdict
        best_rank: dict[str, int] = defaultdict(lambda: 10**9)
        any_results: list = []
        for v in variants:
            res = await backend.search(
                query=v,
                tenant_id=tenant_id,
                k=max(self.config.top_k, 10),
            )
            for i, r in enumerate(res):
                if i < best_rank[r.event_id]:
                    best_rank[r.event_id] = i
                if not any_results or all(
                    ar.event_id != r.event_id for ar in any_results
                ):
                    any_results.append(r)
        # Order results by best rank-across-variants ascending.
        any_results.sort(key=lambda r: best_rank[r.event_id])
        results = any_results
        retrieved_ids = [r.event_id for r in results]
        top5 = set(retrieved_ids[:5])
        top10 = set(retrieved_ids[:10])

        recall_5 = len(gold_event_ids & top5) / len(gold_event_ids)
        recall_10 = len(gold_event_ids & top10) / len(gold_event_ids)

        # We pass on recall@10 >= 1.0 (all gold evidence in top-10).
        passed = recall_10 >= 1.0
        return ExampleResult(
            example_id=f"{tenant_id}#{evidence_dia_ids[0]}",
            passed=passed,
            score=recall_10,
            details={
                "category": category,
                "question": question[:120],
                "evidence": evidence_dia_ids,
                "recall_at_5": recall_5,
                "recall_at_10": recall_10,
                "k_retrieved": len(results),
            },
        )
