"""Core types shared between backends, benchmarks, and the runner.

Designed to be the smallest surface that lets us swap Mem0, Zep, Mastra, MemPalace,
gbrain, Ditto's own backend, and a reference stub on the same fixtures.
"""

from __future__ import annotations

import hashlib
import json
from dataclasses import dataclass, field
from typing import Any


def _canonical_json(obj: Any) -> bytes:
    """Deterministic JSON for content-addressing.

    Sorted keys, no whitespace, UTF-8. Matches what the Ditto Rust core will produce
    so that event_ids computed here are identical to the ones the production harness
    will mint.
    """
    return json.dumps(obj, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode("utf-8")


def content_address(payload: dict[str, Any]) -> str:
    """SHA-256 hex of the canonical JSON encoding of payload."""
    return hashlib.sha256(_canonical_json(payload)).hexdigest()


@dataclass(frozen=True)
class Event:
    """A unit written to memory.

    `event_id` is the SHA-256 of canonical JSON of `payload`. This is the content
    address; identical payloads collide on `event_id`, making writes idempotent.

    `prev_event_id` is the hash-chain link — receipt verification walks this back.
    Backends that don't support chains may store it as metadata or ignore it.
    """

    tenant_id: str
    source_id: str
    payload: dict[str, Any]
    timestamp: float  # unix seconds, float for sub-second precision
    prev_event_id: str | None = None
    event_id: str = field(default="")

    def __post_init__(self) -> None:
        if not self.event_id:
            object.__setattr__(self, "event_id", content_address(self.payload))


@dataclass(frozen=True)
class Receipt:
    """What a backend returns from `write`.

    A backend without signing leaves `signature` as `None`; `verify` for such
    receipts is a no-op that returns True. Backends with signing produce a
    detached Ed25519 signature over `(event_id, prev_event_id, timestamp)`.
    """

    event_id: str
    prev_event_id: str | None
    timestamp: float
    signature: str | None = None  # hex


@dataclass(frozen=True)
class SearchResult:
    """One ranked record from a search.

    `source_event_ids` is the provenance trail — which episodic events back this
    record. Required for Ditto-Provenance-Bench. Backends without provenance
    tracking return `[]` and score 0 on that benchmark.
    """

    event_id: str
    content: str
    score: float
    source_event_ids: list[str]
    metadata: dict[str, Any] = field(default_factory=dict)
