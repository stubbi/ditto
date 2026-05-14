"""CLI: `ditto-eval run --benchmark <name> --backend <name>`."""

from __future__ import annotations

import asyncio
from pathlib import Path

import click

from ditto_eval.backends import StubBackend
from ditto_eval.backends.base import MemoryBackend
from ditto_eval.benchmarks import ProvenanceBench
from ditto_eval.benchmarks.base import Benchmark
from ditto_eval.runner import run_benchmark

BACKENDS: dict[str, type[MemoryBackend]] = {
    "stub": StubBackend,
}

BENCHMARKS: dict[str, type[Benchmark]] = {
    "provenance": ProvenanceBench,
}

DEFAULT_FIXTURES = {
    "provenance": Path(__file__).parent.parent / "fixtures" / "provenance" / "v0.yaml",
}


@click.group()
def main() -> None:
    """ditto-eval: benchmark agent memory systems."""


@main.command()
@click.option("--benchmark", "-b", type=click.Choice(list(BENCHMARKS)), required=True)
@click.option("--backend", "-k", type=click.Choice(list(BACKENDS)), required=True)
@click.option("--fixture", "-f", type=click.Path(exists=True, path_type=Path), default=None)
@click.option("--results-dir", "-r", type=click.Path(path_type=Path), default=Path("results"))
def run(benchmark: str, backend: str, fixture: Path | None, results_dir: Path) -> None:
    """Run a benchmark against a backend."""
    fixture_path = fixture or DEFAULT_FIXTURES[benchmark]
    bench_cls = BENCHMARKS[benchmark]
    backend_cls = BACKENDS[backend]

    async def _go() -> None:
        b = bench_cls(fixture_path)
        bk = backend_cls()
        try:
            result = await run_benchmark(b, bk, results_dir=results_dir)
        finally:
            await bk.close()
        click.echo(f"{result.benchmark} on {result.backend}:")
        click.echo(f"  passed: {result.passed}/{result.total}")
        click.echo(f"  score:  {result.score:.3f}")
        for e in result.examples:
            mark = "PASS" if e.passed else "FAIL"
            click.echo(f"    [{mark}] {e.example_id}  score={e.score:.2f}  {e.details}")

    asyncio.run(_go())


@main.command(name="list")
def list_things() -> None:
    """List available backends and benchmarks."""
    click.echo("backends:")
    for name in BACKENDS:
        click.echo(f"  - {name}")
    click.echo("benchmarks:")
    for name in BENCHMARKS:
        click.echo(f"  - {name}")


if __name__ == "__main__":
    main()
