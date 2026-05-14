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
    "You rewrite a user question for memory retrieval. Return BOTH:\n"
    "  • 2 alternate phrasings of the question (different vocabulary,\n"
    "    same meaning)\n"
    "  • 2 hypothetical answer phrases — sentences that might appear in\n"
    "    the source memory and would answer the question if found. Use\n"
    "    vocabulary the answer turn might actually contain. This is the\n"
    "    HyDE technique (Gao et al. 2022).\n"
    "Example: question = 'What did Caroline research?' →\n"
    "  variants: ['What topics was Caroline studying?',\n"
    "             'What did Caroline look into?'],\n"
    "  hypothetical_answers: ['Caroline was researching adoption.',\n"
    "                         'Caroline is studying counseling.']\n"
    "Return JSON: {\"variants\": [...], \"hypothetical_answers\": [...]}."
)


async def expand_query(llm: LlmClient, question: str, model: str | None = None) -> list[str]:
    """Return [original, *alt_phrasings, *hypothetical_answer_phrases].

    Always includes the original question first so we never lose its
    signal. HyDE phrasings are searched alongside paraphrases — they
    match evidence turns that share answer vocabulary even when no
    question vocab overlaps.
    """
    prompt = f"Question: {question}\n\nReturn JSON."
    try:
        data = await llm.chat_json(
            [
                ChatMessage(role="system", content=EXPANSION_SYSTEM),
                ChatMessage(role="user", content=prompt),
            ],
            model=model,
            max_tokens=300,
        )
        variants = data.get("variants") or []
        hyde = data.get("hypothetical_answers") or []
        seen = {question.lower()}
        out = [question]
        for v in list(variants) + list(hyde):
            if not isinstance(v, str):
                continue
            v = v.strip()
            if not v or v.lower() in seen:
                continue
            seen.add(v.lower())
            out.append(v)
            if len(out) >= 5:  # original + 2 variants + 2 hyde
                break
        return out
    except Exception:  # noqa: BLE001 — bench-level catch
        return [question]
