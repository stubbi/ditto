"""Thin OpenRouter / OpenAI-compatible chat-completions client.

Kept dependency-free (only httpx) so the eval harness doesn't grow heavy
SDK deps. Reads `OPENROUTER_API_KEY` from env. The base URL and model
default to OpenRouter values but can be overridden.

Why our own client and not `openai`: the eval harness needs to swap
between providers (OpenRouter, OpenAI, Anthropic direct) for cost and
availability reasons; a small in-process client lets us A/B providers
without leaking a vendor SDK contract into the benchmark layer.
"""

from __future__ import annotations

import asyncio
import json
import os
from dataclasses import dataclass

import httpx


class LlmError(RuntimeError):
    """Raised when an LLM call fails after retries."""


@dataclass
class ChatMessage:
    role: str  # "system" | "user" | "assistant"
    content: str


class LlmClient:
    """Minimal async chat-completions client.

    Concurrency: callers wrap calls in `asyncio.gather` themselves; this
    class doesn't manage a worker pool. Rate-limiting is delegated to the
    upstream API and a small exponential backoff on 429/5xx.
    """

    def __init__(
        self,
        *,
        api_key: str | None = None,
        base_url: str = "https://openrouter.ai/api/v1",
        model: str = "openai/gpt-4o-mini",
        timeout: float = 60.0,
        max_retries: int = 4,
    ) -> None:
        self.api_key = api_key or os.environ.get("OPENROUTER_API_KEY")
        if not self.api_key:
            raise LlmError("OPENROUTER_API_KEY not set and no api_key arg")
        self.base_url = base_url.rstrip("/")
        self.model = model
        self.timeout = timeout
        self.max_retries = max_retries
        self._client = httpx.AsyncClient(timeout=timeout)

    async def close(self) -> None:
        await self._client.aclose()

    async def chat(
        self,
        messages: list[ChatMessage],
        *,
        model: str | None = None,
        temperature: float = 0.0,
        max_tokens: int | None = None,
        response_format: dict | None = None,
    ) -> str:
        """One chat-completion call. Returns the assistant content string.

        `response_format={"type": "json_object"}` constrains the model to
        emit valid JSON — used by the LLM extractor and the judge.
        """
        payload: dict = {
            "model": model or self.model,
            "messages": [{"role": m.role, "content": m.content} for m in messages],
            "temperature": temperature,
        }
        if max_tokens is not None:
            payload["max_tokens"] = max_tokens
        if response_format is not None:
            payload["response_format"] = response_format

        last_err: Exception | None = None
        for attempt in range(self.max_retries):
            try:
                r = await self._client.post(
                    f"{self.base_url}/chat/completions",
                    headers={
                        "Authorization": f"Bearer {self.api_key}",
                        "Content-Type": "application/json",
                    },
                    json=payload,
                )
                if r.status_code in (429, 500, 502, 503, 504):
                    raise httpx.HTTPStatusError(
                        f"retryable {r.status_code}", request=r.request, response=r
                    )
                r.raise_for_status()
                data = r.json()
                return data["choices"][0]["message"]["content"]
            except (httpx.HTTPStatusError, httpx.TimeoutException, httpx.ReadError) as e:
                last_err = e
                # Exponential backoff: 1, 2, 4, 8s.
                await asyncio.sleep(2**attempt)
                continue
        raise LlmError(f"chat failed after {self.max_retries} attempts: {last_err}")

    async def chat_json(
        self,
        messages: list[ChatMessage],
        *,
        model: str | None = None,
        max_tokens: int | None = None,
    ) -> dict:
        """JSON-mode chat. Parses the response; raises on invalid JSON."""
        raw = await self.chat(
            messages,
            model=model,
            max_tokens=max_tokens,
            response_format={"type": "json_object"},
        )
        try:
            return json.loads(raw)
        except json.JSONDecodeError as e:
            raise LlmError(f"non-JSON response: {raw[:200]}") from e
