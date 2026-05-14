"""Reference in-memory backend.

Not a real memory system — no semantic search, no embeddings, no KG. It exists to:
  1. Validate the eval harness end-to-end without external dependencies
  2. Establish a control floor for benchmark scores ("can you beat a substring scan?")
  3. Serve as the simplest possible reference implementation of the protocol

Retrieval is naive substring matching with a recency tiebreaker.
"""

from __future__ import annotations

from collections.abc import Sequence

from ditto_eval.backends.base import MemoryBackend, SearchMode
from ditto_eval.types import Event, Receipt, SearchResult


class StubBackend(MemoryBackend):
    name = "stub"

    def __init__(self) -> None:
        # tenant_id -> list[Event], append-only within a tenant
        self._store: dict[str, list[Event]] = {}

    async def write(self, event: Event) -> Receipt:
        events = self._store.setdefault(event.tenant_id, [])
        # idempotent on event_id — skip if already present
        if not any(e.event_id == event.event_id for e in events):
            events.append(event)
        return Receipt(
            event_id=event.event_id,
            prev_event_id=event.prev_event_id,
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
        events = self._store.get(tenant_id, [])
        candidates = [
            e for e in events
            if sources is None or e.source_id in sources
        ]
        q = query.lower()
        scored: list[tuple[float, Event]] = []
        for e in candidates:
            text = self._render(e)
            hits = text.lower().count(q)
            if hits == 0:
                continue
            # base score = match count; tiebreak on recency (negative because newer = higher)
            score = float(hits) + (e.timestamp / 1e12)
            scored.append((score, e))
        scored.sort(key=lambda x: x[0], reverse=True)
        return [
            SearchResult(
                event_id=e.event_id,
                content=self._render(e),
                score=score,
                source_event_ids=[e.event_id],  # stub: event is its own provenance
                metadata={"source_id": e.source_id, "timestamp": e.timestamp},
            )
            for score, e in scored[:k]
        ]

    async def verify(self, receipt: Receipt) -> bool:
        return True

    async def reset(self, tenant_id: str) -> None:
        self._store.pop(tenant_id, None)

    @staticmethod
    def _render(event: Event) -> str:
        """Render an event payload as a string for naive matching."""
        if "content" in event.payload and isinstance(event.payload["content"], str):
            return event.payload["content"]
        return str(event.payload)
