"""Benchmark protocol and result types."""

from __future__ import annotations

from abc import ABC, abstractmethod
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

from ditto_eval.backends.base import MemoryBackend


@dataclass
class ExampleResult:
    example_id: str
    passed: bool
    score: float  # 0.0 to 1.0
    details: dict[str, Any] = field(default_factory=dict)


@dataclass
class BenchmarkResult:
    benchmark: str
    backend: str
    fixture_version: str
    total: int
    passed: int
    score: float  # aggregate, 0.0 to 1.0
    examples: list[ExampleResult] = field(default_factory=list)
    metadata: dict[str, Any] = field(default_factory=dict)

    @property
    def pass_rate(self) -> float:
        return self.passed / self.total if self.total else 0.0


class Benchmark(ABC):
    name: str = "unspecified"

    def __init__(self, fixture: Path) -> None:
        self.fixture = fixture

    @abstractmethod
    async def run(self, backend: MemoryBackend) -> BenchmarkResult: ...
