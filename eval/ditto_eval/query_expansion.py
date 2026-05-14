"""Query expansion via an OpenRouter-hosted LLM.

For a single question, ask the LLM for 2-3 alternate phrasings that
emphasize different aspects (entity, action, object). Searching each
phrasing and unioning the results widens the recall pool — especially
useful on single-hop questions where the question's vocabulary
doesn't match the evidence turn's vocabulary.

Example:
  "What did Caroline research?"
  → [
      "What did Caroline research?",
      "What topics or fields was Caroline studying?",
      "What did Caroline look into or investigate?",
    ]

The original question is always included as the first variant so we
never lose its signal. Expansion is best-effort: on any LLM error we
return just the original.
"""

from __future__ import annotations

import json

from ditto_eval.llm import ChatMessage, LlmClient

EXPANSION_SYSTEM = (
    "You rewrite a user question into 2-3 alternate phrasings that "
    "emphasize different aspects: the entity, the action verb, the "
    "object/topic. Do not change the meaning. Use diverse vocabulary "
    "so the rewrites are useful for keyword/semantic retrieval. "
    "Return JSON: {\"variants\": [<phrasings>]}. Do not include the "
    "original question in your output."
)


async def expand_query(llm: LlmClient, question: str, model: str | None = None) -> list[str]:
    """Return [original, *alt_phrasings]. Always includes original first."""
    prompt = f"Question: {question}\n\nReturn JSON with 2-3 alternate phrasings."
    try:
        data = await llm.chat_json(
            [
                ChatMessage(role="system", content=EXPANSION_SYSTEM),
                ChatMessage(role="user", content=prompt),
            ],
            model=model,
            max_tokens=200,
        )
        variants = data.get("variants") or []
        # Defensive: keep only strings, dedupe, cap at 3.
        seen = {question.lower()}
        out = [question]
        for v in variants:
            if not isinstance(v, str):
                continue
            v = v.strip()
            if not v or v.lower() in seen:
                continue
            seen.add(v.lower())
            out.append(v)
            if len(out) >= 4:
                break
        return out
    except Exception:  # noqa: BLE001 — bench-level catch
        return [question]
