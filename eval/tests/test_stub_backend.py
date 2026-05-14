"""Smoke tests for the eval harness on the stub backend.

These don't validate that the stub backend is *good* — it's a substring scanner.
They validate that the harness orchestrates correctly end-to-end.
"""

from __future__ import annotations

from pathlib import Path

import pytest

from ditto_eval.backends import StubBackend
from ditto_eval.benchmarks import ProvenanceBench
from ditto_eval.runner import run_benchmark
from ditto_eval.types import Event, content_address


def test_content_address_is_deterministic() -> None:
    a = content_address({"content": "hi", "ts": 1})
    b = content_address({"ts": 1, "content": "hi"})
    assert a == b
    assert len(a) == 64


def test_event_id_matches_payload_hash() -> None:
    e = Event(
        tenant_id="t",
        source_id="s",
        payload={"content": "hello"},
        timestamp=1.0,
    )
    assert e.event_id == content_address({"content": "hello"})


@pytest.mark.asyncio
async def test_stub_write_idempotent() -> None:
    b = StubBackend()
    e = Event(tenant_id="t", source_id="s", payload={"content": "hi"}, timestamp=1.0)
    r1 = await b.write(e)
    r2 = await b.write(e)
    assert r1.event_id == r2.event_id
    results = await b.search("hi", tenant_id="t")
    assert len(results) == 1


@pytest.mark.asyncio
async def test_stub_runs_provenance_benchmark() -> None:
    """End-to-end smoke test.

    The stub is a substring scanner; it cannot semantically match "When was the
    user born?" to "User mentioned their birthday is March 14, 1992." Expected
    score is 0.0 — this confirms Provenance-Bench is grading semantics, not
    string matching, and gives us a credible control floor.
    """
    fixture = Path(__file__).parent.parent / "fixtures" / "provenance" / "v0.yaml"
    bench = ProvenanceBench(fixture)
    backend = StubBackend()
    result = await run_benchmark(bench, backend, results_dir=None)
    assert result.total == 3
    assert result.benchmark == "ditto-provenance-bench"
    assert result.backend == "stub"
    # control floor: naive substring matching scores 0; any real backend must beat this
    assert result.score == 0.0
    assert all(not ex.details["leak"] for ex in result.examples)
