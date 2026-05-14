"""Benchmark runners.

Each benchmark is a `Benchmark` subclass that loads a fixture, drives a
`MemoryBackend` through write+search, and returns a `BenchmarkResult`.

Implemented:
- `ProvenanceBench` — Ditto-Provenance-Bench, our original fixture
- `LocomoBench` — LoCoMo (Maharana et al. 2024), the canonical
  long-conversation memory benchmark. 10 multi-session dialogs, ~2000
  questions across 5 categories.

Forthcoming:
- `LongMemEval` — public ICLR 2025 benchmark
- `BEAM` — public ICLR 2026 benchmark, 2k questions, 1M and 10M token scales
- `IsolationBench` — Ditto-Isolation-Bench, adversarial multi-tenant
- `CrashBench` — kill -9 mid-write, verify durability
"""

from ditto_eval.benchmarks.base import Benchmark, BenchmarkResult, ExampleResult
from ditto_eval.benchmarks.locomo import LocomoBench, LocomoConfig
from ditto_eval.benchmarks.locomo_retrieval import LocomoRetrievalBench
from ditto_eval.benchmarks.provenance import ProvenanceBench

__all__ = [
    "Benchmark",
    "BenchmarkResult",
    "ExampleResult",
    "LocomoBench",
    "LocomoConfig",
    "LocomoRetrievalBench",
    "ProvenanceBench",
]
