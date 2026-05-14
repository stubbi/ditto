"""The contract every memory backend implements.

Keep this surface small. Adding a method to this ABC means rewriting every adapter.
"""

from __future__ import annotations

from abc import ABC, abstractmethod
from collections.abc import Sequence
from typing import Literal

from ditto_eval.types import Event, Receipt, SearchResult

SearchMode = Literal["cheap", "standard", "deep"]


class MemoryBackend(ABC):
    """Contract for any memory system the eval harness can run benchmarks against.

    Tenants: backends that don't natively support multi-tenancy treat `tenant_id`
    as a namespace and rely on the benchmark to reset between runs.

    Sources: backends that don't support source isolation ignore `source_id` and
    return results across all sources.

    Provenance: backends without provenance tracking return empty
    `source_event_ids` on `SearchResult` — they score 0 on Ditto-Provenance-Bench
    but can still pass LongMemEval / BEAM.
    """

    name: str = "unspecified"

    @abstractmethod
    async def write(self, event: Event) -> Receipt:
        """Commit a single event. Must be idempotent on `event.event_id`."""

    @abstractmethod
    async def search(
        self,
        query: str,
        tenant_id: str,
        sources: Sequence[str] | None = None,
        k: int = 10,
        mode: SearchMode = "standard",
    ) -> list[SearchResult]:
        """Retrieve up to `k` records relevant to `query` for `tenant_id`.

        `sources` restricts to specific source_ids; `None` means all sources.
        `mode` is a cost/quality knob — backends that don't have modes should
        treat all modes as their single retrieval path.
        """

    @abstractmethod
    async def verify(self, receipt: Receipt) -> bool:
        """Verify a receipt. Backends without signing return True for any receipt."""

    @abstractmethod
    async def reset(self, tenant_id: str) -> None:
        """Wipe all data for `tenant_id`. Used between benchmark runs."""

    async def consolidate(self, tenant_id: str, mode: str = "dream") -> dict | None:
        """Optionally trigger a consolidation pass. Default no-op for
        backends without one. Used by long-conversation benchmarks to
        run a dream sweep after batch ingest, populating the NC-graph
        before QA. Returns the backend's report dict if available.
        """
        return None

    async def close(self) -> None:
        """Release resources. Default no-op; backends with connections override."""
