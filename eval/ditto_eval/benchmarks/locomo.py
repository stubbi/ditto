"""LoCoMo (Long Conversation Memory) benchmark.

LoCoMo (Maharana et al. 2024) is the canonical benchmark for long-term
conversational memory. 10 multi-session dialogs between two speakers,
each spanning ~20+ sessions over months of in-narrative time, with
~200 questions per conversation across five categories:

  1: single-hop factual lookup
  2: temporal reasoning ("when did X")
  3: multi-hop synthesis across sessions
  4: open-domain / commonsense extension
  5: adversarial — must abstain when no evidence

Source: snap-stanford/locomo (academic release). Convenience mirror at
github.com/Backboard-io/Backboard-Locomo-Benchmark (MIT-licensed fork)
hosts the dataset as a single JSON file. We download once into a
gitignored cache to avoid bundling.

The bench loads conversations, writes every dialog turn as an episodic
event (`source_id = speaker`, content = turn text, timestamp parsed
from the session's date), then for each question:

  - searches the tenant's memory with the question as the query
  - feeds top-K results to a QA-solver LLM
  - judges the predicted answer against the gold answer with a judge LLM

Subset support: pass `max_questions_per_category` to stratify-sample
each conversation's QA set. Useful for cheap iteration cycles before
the full ~2000-question sweep.
"""

from __future__ import annotations

import asyncio
import json
import re
from dataclasses import dataclass
from datetime import datetime
from pathlib import Path

import httpx

from ditto_eval.backends.base import MemoryBackend
from ditto_eval.benchmarks.base import Benchmark, BenchmarkResult, ExampleResult
from ditto_eval.llm import ChatMessage, LlmClient
from ditto_eval.types import Event

DATASET_URL = (
    "https://raw.githubusercontent.com/Backboard-io/Backboard-Locomo-Benchmark/"
    "main/locomo_dataset.json"
)

CATEGORY_NAMES = {
    1: "single_hop",
    2: "temporal",
    3: "multi_hop",
    4: "open_domain",
    5: "adversarial",
}


def _cache_path(root: Path) -> Path:
    return root / "locomo_dataset.json"


def fetch_dataset(cache_dir: Path) -> Path:
    """Download the dataset on first use; reuse the cached copy after.

    Cache dir is gitignored — see eval/.gitignore — because the raw
    dataset is large and the upstream is the authoritative source.
    """
    cache_dir.mkdir(parents=True, exist_ok=True)
    path = _cache_path(cache_dir)
    if not path.exists():
        with httpx.Client(timeout=120.0) as c:
            r = c.get(DATASET_URL)
            r.raise_for_status()
            path.write_bytes(r.content)
    return path


def parse_session_datetime(s: str) -> float:
    """Parse 'h:MMam on D Month, YYYY' into a unix timestamp.

    LoCoMo timestamps look like '1:56 pm on 8 May, 2023'. They're not ISO
    so we hand-roll the parse. Returns float seconds since epoch (UTC).
    Falls back to a stable hash-based offset on parse failure — the
    bench needs monotonically increasing timestamps within a
    conversation, not actual wall-clock accuracy.
    """
    # "1:56 pm on 8 May, 2023" → "8 May 2023 1:56 pm"
    m = re.match(r"\s*(\d{1,2}:\d{2}\s*[ap]m)\s+on\s+(\d{1,2})\s+(\w+),?\s+(\d{4})", s, re.I)
    if not m:
        return float(abs(hash(s)) % 10_000_000_000)
    hm, day, month, year = m.groups()
    try:
        dt = datetime.strptime(f"{day} {month} {year} {hm.upper()}", "%d %B %Y %I:%M %p")
        return dt.timestamp()
    except ValueError:
        return float(abs(hash(s)) % 10_000_000_000)


@dataclass
class LocomoConfig:
    """Knobs for the LoCoMo bench.

    `max_questions_per_category`: if set, stratify-sample to N questions
    per category per conversation. Use 5 or 10 for dev cycles; None for
    the full ~2000-question sweep.

    `conversations`: subset of conversation indices (0..9) to run.
    `None` runs all 10.

    `top_k`: retrieval k handed to the backend's `search`.

    `qa_model` / `judge_model`: OpenRouter model IDs. Defaults are cheap.
    """

    max_questions_per_category: int | None = None
    conversations: list[int] | None = None
    top_k: int = 10
    qa_model: str = "openai/gpt-4o-mini"
    judge_model: str = "openai/gpt-4o-mini"
    concurrency: int = 8


QA_SYSTEM = (
    "You are a careful question-answering assistant. Given retrieved "
    "memory fragments from a multi-session conversation between two "
    "people, answer the user's question. Use ONLY the provided context. "
    "If the context is insufficient, answer 'No information'. Keep your "
    "answer concise — a single short phrase or sentence."
)

JUDGE_SYSTEM = (
    "You are an answer-grading assistant. Given a question, a gold "
    "reference answer, and a predicted answer, decide whether the "
    "predicted answer is semantically equivalent to the gold answer. "
    "Equivalence is loose: dates can be reformatted, names can be "
    "abbreviated, partial mentions of the right entity count. "
    "Adversarial 'No information' / 'I don't know' counts as correct "
    "ONLY when the gold answer is also 'No information' or similar. "
    "Respond with a single JSON object: {\"correct\": true|false}."
)


def _build_qa_prompt(question: str, results: list) -> str:
    if not results:
        ctx = "(no relevant memories retrieved)"
    else:
        lines = []
        for i, r in enumerate(results, 1):
            lines.append(f"[{i}] {r.content}")
        ctx = "\n".join(lines)
    return f"Context:\n{ctx}\n\nQuestion: {question}\nAnswer:"


def _build_judge_prompt(question: str, gold: str, predicted: str) -> str:
    return (
        f"Question: {question}\n"
        f"Gold answer: {gold}\n"
        f"Predicted answer: {predicted}\n"
        "Is the predicted answer correct? Return JSON."
    )


def _stratify(qa: list, max_per_cat: int | None) -> list:
    if max_per_cat is None:
        return qa
    buckets: dict[int, list] = {}
    for q in qa:
        buckets.setdefault(q["category"], []).append(q)
    out: list = []
    for cat, items in buckets.items():
        out.extend(items[:max_per_cat])
    return out


class LocomoBench(Benchmark):
    name = "locomo"

    def __init__(self, fixture: Path, config: LocomoConfig | None = None) -> None:
        # `fixture` is the dataset cache directory; downloads on first use.
        super().__init__(fixture)
        self.config = config or LocomoConfig()

    async def run(self, backend: MemoryBackend) -> BenchmarkResult:
        cache_dir = self.fixture
        ds_path = fetch_dataset(cache_dir)
        with ds_path.open() as f:
            dataset = json.load(f)

        targets = (
            range(len(dataset))
            if self.config.conversations is None
            else self.config.conversations
        )

        llm = LlmClient()
        examples: list[ExampleResult] = []
        by_category: dict[str, list[bool]] = {v: [] for v in CATEGORY_NAMES.values()}

        try:
            for conv_idx in targets:
                conv = dataset[conv_idx]
                tenant_id = f"locomo-{conv['sample_id']}"
                await backend.reset(tenant_id)

                # Write every dialog turn as an event. Source = speaker;
                # timestamp = session_date_time + turn_index * 60s so
                # ordering within a session is monotonic.
                conversation = conv["conversation"]
                prev: str | None = None
                for skey in sorted(
                    [k for k in conversation if re.fullmatch(r"session_\d+", k)],
                    key=lambda s: int(s.split("_")[1]),
                ):
                    dt_key = f"{skey}_date_time"
                    base_ts = parse_session_datetime(conversation.get(dt_key, ""))
                    turns = conversation[skey]
                    if not isinstance(turns, list):
                        continue
                    for i, turn in enumerate(turns):
                        # Some turns carry image captions; we collapse to text.
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
                        prev = event.event_id

                # Run QA — concurrency-limited to avoid hammering OpenRouter.
                qa = _stratify(conv["qa"], self.config.max_questions_per_category)
                sem = asyncio.Semaphore(self.config.concurrency)

                async def one(q: dict, conv_id: str = tenant_id) -> ExampleResult:
                    async with sem:
                        return await self._one_qa(backend, llm, conv_id, q)

                conv_results = await asyncio.gather(*[one(q) for q in qa])
                for r in conv_results:
                    examples.append(r)
                    cat_name = r.details.get("category", "unknown")
                    if cat_name in by_category:
                        by_category[cat_name].append(r.passed)

                await backend.reset(tenant_id)
        finally:
            await llm.close()

        per_cat = {
            cat: (sum(vs) / len(vs) if vs else None) for cat, vs in by_category.items()
        }
        passed = sum(1 for e in examples if e.passed)
        score = passed / len(examples) if examples else 0.0
        return BenchmarkResult(
            benchmark=self.name,
            backend=backend.name,
            fixture_version="locomo10",
            total=len(examples),
            passed=passed,
            score=score,
            examples=examples,
            metadata={
                "per_category_accuracy": per_cat,
                "qa_model": self.config.qa_model,
                "judge_model": self.config.judge_model,
                "top_k": self.config.top_k,
                "max_questions_per_category": self.config.max_questions_per_category,
            },
        )

    async def _one_qa(
        self,
        backend: MemoryBackend,
        llm: LlmClient,
        tenant_id: str,
        q: dict,
    ) -> ExampleResult:
        question = q["question"]
        # Category 5 (adversarial) ships an `adversarial_answer` — the
        # plausible wrong answer the model might confabulate — and no
        # `answer` field. The gold behavior is to abstain. We encode
        # "No information" as the gold so the judge can grade
        # abstention vs. confabulation symmetrically with the other
        # categories.
        if "answer" in q:
            gold = str(q["answer"])
            adversarial = False
        else:
            gold = "No information"
            adversarial = True
        category = CATEGORY_NAMES.get(q["category"], "unknown")

        results = await backend.search(
            query=question,
            tenant_id=tenant_id,
            k=self.config.top_k,
            mode="standard",
        )
        prompt = _build_qa_prompt(question, results)
        try:
            predicted = (await llm.chat(
                [
                    ChatMessage(role="system", content=QA_SYSTEM),
                    ChatMessage(role="user", content=prompt),
                ],
                model=self.config.qa_model,
                max_tokens=200,
            )).strip()
        except Exception as e:  # noqa: BLE001 — bench-level catch
            return ExampleResult(
                example_id=f"{tenant_id}#{q.get('evidence', ['?'])[0]}",
                passed=False,
                score=0.0,
                details={"category": category, "error": f"qa: {e}", "gold": gold},
            )

        try:
            judged = await llm.chat_json(
                [
                    ChatMessage(role="system", content=JUDGE_SYSTEM),
                    ChatMessage(role="user", content=_build_judge_prompt(question, gold, predicted)),
                ],
                model=self.config.judge_model,
                max_tokens=20,
            )
            correct = bool(judged.get("correct", False))
        except Exception as e:  # noqa: BLE001
            return ExampleResult(
                example_id=f"{tenant_id}#{q.get('evidence', ['?'])[0]}",
                passed=False,
                score=0.0,
                details={"category": category, "error": f"judge: {e}", "gold": gold, "predicted": predicted},
            )

        return ExampleResult(
            example_id=f"{tenant_id}#{q.get('evidence', ['?'])[0]}",
            passed=correct,
            score=1.0 if correct else 0.0,
            details={
                "category": category,
                "question": question,
                "gold": gold,
                "predicted": predicted,
                "k_retrieved": len(results),
                "adversarial": adversarial,
            },
        )
