"""Orchestrator: load benchmark, run it against backend, record result."""

from __future__ import annotations

import json
from dataclasses import asdict
from datetime import UTC, datetime
from pathlib import Path

from ditto_eval.backends.base import MemoryBackend
from ditto_eval.benchmarks.base import Benchmark, BenchmarkResult


async def run_benchmark(
    benchmark: Benchmark,
    backend: MemoryBackend,
    results_dir: Path | None = None,
) -> BenchmarkResult:
    result = await benchmark.run(backend)
    if results_dir is not None:
        results_dir.mkdir(parents=True, exist_ok=True)
        ts = datetime.now(UTC).strftime("%Y%m%dT%H%M%SZ")
        path = results_dir / f"{result.benchmark}_{result.backend}_{ts}.json"
        with path.open("w") as f:
            json.dump(asdict(result), f, indent=2, sort_keys=True)
    return result
