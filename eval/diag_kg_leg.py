"""Diagnose why the LLM extractor isn't moving recall on LoCoMo.

Ingests conversation 0 with extractor=llm (dream-only) + consolidate
after ingest, then searches a known-failing single-hop question and
prints every result with its leg metadata. Tells us:

  - Did KG-leg results appear at all? → extractor populated the graph
  - Where do they rank vs BM25/vector? → RRF weight problem
  - Do they cite the right events? → entity linking problem
"""

from __future__ import annotations

import asyncio
import json
import os
import re

from ditto_eval.backends import DittoBackend
from ditto_eval.benchmarks.locomo import parse_session_datetime
from ditto_eval.types import Event

PROBE_QUERIES = [
    "What did Caroline research?",
    "What is Caroline's identity?",
    "Where did Caroline move from 4 years ago?",
]


async def main() -> None:
    os.environ["DITTO_EXTRACTOR"] = "llm"
    os.environ["DITTO_EXTRACT_ON_WRITE"] = "0"
    os.environ.setdefault("DITTO_RERANKER", "none")  # don't rerank — we want raw leg attribution

    bk = DittoBackend()
    ds = json.load(open("/tmp/locomo_dataset.json"))
    conv = ds[0]
    tenant_id = "diag-kg-conv-26"
    await bk.reset(tenant_id)

    convo = conv["conversation"]
    prev: str | None = None
    n_written = 0
    for skey in sorted(
        [k for k in convo if re.fullmatch(r"session_\d+", k)],
        key=lambda s: int(s.split("_")[1]),
    ):
        base_ts = parse_session_datetime(convo.get(f"{skey}_date_time", ""))
        turns = convo[skey]
        if not isinstance(turns, list):
            continue
        for i, turn in enumerate(turns):
            text = turn.get("text", "")
            if "img_url" in turn or "blip_caption" in turn:
                cap = turn.get("blip_caption") or turn.get("img_url", "")
                text = f"{text} [image: {cap}]".strip()
            if not text:
                continue
            ev = Event(
                tenant_id=tenant_id,
                source_id=turn.get("speaker", "unknown"),
                payload={"content": text, "dia_id": turn.get("dia_id", "")},
                timestamp=base_ts + i * 60.0,
                prev_event_id=prev,
            )
            await bk.write(ev)
            prev = ev.event_id
            n_written += 1
    print(f"wrote {n_written} events")

    # Run the LLM-extractor dream sweep.
    report = await bk.consolidate(tenant_id, mode="dream")
    print(f"consolidate(dream) → {json.dumps(report, indent=2)[:500]}")

    for q in PROBE_QUERIES:
        print(f"\n=== {q} ===")
        results = await bk.search(query=q, tenant_id=tenant_id, k=30)
        # Sort: keep server order, but tag legs.
        for i, r in enumerate(results[:15]):
            md = r.metadata or {}
            leg = md.get("leg", "?")
            kg_score = md.get("kg_score", "")
            print(
                f"  [{i+1:2}] score={r.score:.4f} leg={leg!s:>6} kg={kg_score!s:>5}  "
                f"{r.content[:80]}"
            )
        # How many KG-leg results in top-15?
        kg_hits = [
            r for r in results[:15]
            if (r.metadata or {}).get("leg") == "kg"
            or (r.metadata or {}).get("kg_score") is not None
        ]
        print(f"  KG-leg results in top-15: {len(kg_hits)}")

    await bk.close()


if __name__ == "__main__":
    asyncio.run(main())
