"""Mem0Backend: thin adapter so we can run our benchmarks against Mem0.

Skeleton-complete. Mem0's `add` path runs an LLM extraction step, so this
adapter requires `OPENAI_API_KEY` (or another configured Mem0 LLM) to
actually execute. CI runs are gated on the env var; if missing, the
adapter raises on construction and the runner records "skipped" rather
than fabricating numbers.

Why we ship a Mem0 adapter at all (when we said "no adapters"):
- Mem0 is the most-cited commercial competitor and publishes the most
  benchmark numbers (LongMemEval 93.4, LoCoMo 91.6, BEAM-1M 64.1).
- Running them under matched conditions on our own fixtures is the
  post-MemPalace-#214 methodology bar.
- One adapter is bounded; five was the rabbit hole we declined.

On *our* benchmarks specifically:
- `Ditto-Provenance-Bench` — Mem0's SearchResults don't carry
  `source_event_ids` (no first-class provenance), so it will score 0 by
  construction. That's the differentiation story Ditto sells.
- `Ditto-Deletion-Bench` (forthcoming) — same shape.
- `LongMemEval` / `BEAM` — Mem0 has real retrieval; Ditto must wait until
  vector + late-interaction rerank land before this is a fair comparison.
"""

from __future__ import annotations

import os
from collections.abc import Sequence
from typing import Any

from ditto_eval.backends.base import MemoryBackend, SearchMode
from ditto_eval.types import Event, Receipt, SearchResult


class Mem0Backend(MemoryBackend):
    name = "mem0"

    def __init__(self) -> None:
        if not (os.environ.get("OPENAI_API_KEY") or os.environ.get("MEM0_LLM_API_KEY")):
            raise RuntimeError(
                "Mem0Backend requires an LLM key (OPENAI_API_KEY or MEM0_LLM_API_KEY). "
                "Mem0's `add()` invokes an LLM at write time for extraction; without "
                "credentials we cannot exercise the real code path. Skip this backend "
                "in CI by gating on the env var."
            )
        try:
            from mem0 import Memory  # type: ignore[import-untyped]
        except ImportError as e:
            raise RuntimeError(
                "Install with `pip install ditto-eval[mem0]` to use this backend."
            ) from e
        self._mem = Memory()

    async def write(self, event: Event) -> Receipt:
        # Mem0 takes a list of messages; we wrap the payload as one user message.
        content = self._render(event.payload)
        # Mem0's `add` is sync; run it directly. For larger tests, wrap in
        # asyncio.to_thread.
        results = self._mem.add(
            messages=[{"role": "user", "content": content}],
            user_id=event.tenant_id,
            metadata={"source": event.source_id, "ditto_event_id": event.event_id},
        )
        # `add` returns a dict with `results`: list of memory ops; we use the
        # first inserted memory id as the receipt identity. Mem0 doesn't sign
        # receipts (no provenance attestation), so `signature` is None.
        added = (results.get("results") or []) if isinstance(results, dict) else results
        first_id = added[0].get("id") if added and isinstance(added[0], dict) else event.event_id
        return Receipt(
            event_id=str(first_id),
            prev_event_id=None,
            timestamp=event.timestamp,
            signature=None,
        )

    async def search(
        self,
        query: str,
        tenant_id: str,
        sources: Sequence[str] | None = None,
        k: int = 10,
        mode: SearchMode = "standard",
    ) -> list[SearchResult]:
        hits = self._mem.search(query=query, user_id=tenant_id, limit=k)
        items = (hits.get("results") or []) if isinstance(hits, dict) else hits
        out: list[SearchResult] = []
        for h in items:
            out.append(
                SearchResult(
                    event_id=str(h.get("id", "")),
                    content=str(h.get("memory") or h.get("content") or ""),
                    score=float(h.get("score", 0.0)),
                    # Mem0 has no first-class provenance — no source_event_ids.
                    source_event_ids=[],
                    metadata=h.get("metadata") or {},
                )
            )
        return out

    async def verify(self, receipt: Receipt) -> bool:
        # Mem0 has no cryptographic provenance. Verification is a no-op success
        # for compatibility — but Provenance-Bench specifically scores 0 here
        # because `source_event_ids` is empty.
        return True

    async def reset(self, tenant_id: str) -> None:
        # Mem0 supports per-user deletion.
        try:
            self._mem.delete_all(user_id=tenant_id)
        except Exception:
            pass

    @staticmethod
    def _render(payload: Any) -> str:
        if isinstance(payload, dict) and isinstance(payload.get("content"), str):
            return payload["content"]
        return str(payload)
