"""DittoBackend: speaks MCP stdio to a `ditto serve` subprocess.

Spawns the Ditto binary in MCP server mode, initializes an MCP client
session, and translates the Python eval harness protocol into MCP tool
calls. This is the production integration path — anything that runs
against this adapter is exercising the same code path real users hit
when they wire Ditto into Claude Code / Cursor / Zed.

Design notes:
- One subprocess per backend instance. Cheap; the binary boots in <50ms.
- Tenant IDs in the eval harness are arbitrary strings; we hash them to
  deterministic UUIDs (uuid5) so Ditto's UUID-typed RLS still works.
- Provenance-Bench writes events with caller-chosen timestamps. We pass
  them through as the payload's `ts` (Ditto's `write_event` sets its own
  `Receipt.timestamp` from now(); for now we keep the protocol simple and
  let Ditto's clock win).
- Reset is a no-op: each Provenance-Bench example uses a fresh tenant_id
  string, so there's no cross-example contamination. Cross-run isolation
  would require a `reset_tenant` MCP tool, which is intentionally not
  exposed (destructive operations should not flow through MCP).
"""

from __future__ import annotations

import json
import os
import uuid
from collections.abc import Sequence
from pathlib import Path
from typing import Any

from mcp import ClientSession, StdioServerParameters
from mcp.client.stdio import stdio_client

from ditto_eval.backends.base import MemoryBackend, SearchMode
from ditto_eval.types import Event, Receipt, SearchResult

# Deterministic namespace so tenant string -> UUID stays stable across runs.
_DITTO_NS = uuid.UUID("00000000-d177-4044-8da0-000000000001")


def _to_uuid(s: str) -> uuid.UUID:
    """Map an arbitrary string identifier to a deterministic UUID v5."""
    return uuid.uuid5(_DITTO_NS, s)


def _default_binary() -> Path:
    """Locate the `ditto` binary built from the workspace.

    Prefers `DITTO_BINARY` env var; falls back to `target/release/ditto`
    relative to the repo root, then `target/debug/ditto`.
    """
    if env := os.environ.get("DITTO_BINARY"):
        return Path(env)
    repo_root = Path(__file__).resolve().parents[3]
    for cand in (
        repo_root / "target" / "release" / "ditto",
        repo_root / "target" / "debug" / "ditto",
    ):
        if cand.exists():
            return cand
    raise RuntimeError(
        f"ditto binary not found under {repo_root}/target/{{release,debug}}/ditto. "
        f"Run `cargo build --release --bin ditto` from the repo root, or set "
        f"DITTO_BINARY to an explicit path."
    )


class DittoBackend(MemoryBackend):
    """Eval-harness adapter that drives Ditto via MCP stdio."""

    name = "ditto"

    def __init__(self, binary: Path | None = None) -> None:
        self._binary = binary or _default_binary()
        self._stack: Any = None
        self._session: ClientSession | None = None
        # Per-backend scope; the eval harness doesn't supply one. Stable so
        # provenance and source_ids round-trip across operations.
        self._scope = uuid.uuid4()

    async def _ensure_session(self) -> ClientSession:
        if self._session is not None:
            return self._session

        from contextlib import AsyncExitStack

        self._stack = AsyncExitStack()
        params = StdioServerParameters(
            command=str(self._binary),
            args=["serve"],
            env={**os.environ},
        )
        read, write = await self._stack.enter_async_context(stdio_client(params))
        session = await self._stack.enter_async_context(ClientSession(read, write))
        await session.initialize()
        self._session = session
        return session

    async def write(self, event: Event) -> Receipt:
        session = await self._ensure_session()
        tenant_uuid = _to_uuid(event.tenant_id)
        # Ditto's MCP write_event computes event_id from canonical(payload).
        # The Python event_id is computed the same way (the test fixture
        # `event_id_matches_python_fixture` guarantees the hash matches).
        result = await session.call_tool(
            "write_event",
            {
                "tenant": str(tenant_uuid),
                "scope": str(self._scope),
                "source": event.source_id,
                "slot": "episodic_index",
                "payload": event.payload,
            },
        )
        receipt_dict = _first_json(result)
        return Receipt(
            event_id=receipt_dict["event_id"],
            prev_event_id=receipt_dict.get("prev_event_id"),
            timestamp=event.timestamp,
            signature=receipt_dict.get("signature"),
        )

    async def search(
        self,
        query: str,
        tenant_id: str,
        sources: Sequence[str] | None = None,
        k: int = 10,
        mode: SearchMode = "standard",
    ) -> list[SearchResult]:
        session = await self._ensure_session()
        args: dict[str, Any] = {
            "tenant": str(_to_uuid(tenant_id)),
            "query": query,
            "k": k,
            "mode": mode,
        }
        if sources is not None:
            args["sources"] = list(sources)
        result = await session.call_tool("search", args)
        items = _first_json(result) or []
        out: list[SearchResult] = []
        for r in items:
            metadata = r.get("metadata") or {}
            out.append(
                SearchResult(
                    event_id=r["event_id"],
                    content=r.get("content", ""),
                    score=float(r.get("score", 0.0)),
                    source_event_ids=list(r.get("source_event_ids") or []),
                    metadata=metadata,
                )
            )
        return out

    async def verify(self, receipt: Receipt) -> bool:
        session = await self._ensure_session()
        receipt_dict = {
            "event_id": receipt.event_id,
            "prev_event_id": receipt.prev_event_id,
            "tenant_id": "00000000-0000-0000-0000-000000000000",  # unused
            "source_id": "",
            "timestamp": "1970-01-01T00:00:00Z",
            "schema_version": 1,
            "signature": receipt.signature,
        }
        result = await session.call_tool("verify_receipt", {"receipt": receipt_dict})
        payload = _first_json(result) or {}
        return bool(payload.get("valid", False))

    async def reset(self, tenant_id: str) -> None:
        # Intentional no-op. The eval harness uses unique tenant_ids per
        # example, so cross-example isolation is already guaranteed. A
        # destructive `reset_tenant` MCP tool is deliberately not exposed.
        return None

    async def close(self) -> None:
        if self._stack is not None:
            await self._stack.aclose()
            self._stack = None
            self._session = None


def _first_json(result: Any) -> Any:
    """Extract the first text-content payload from an MCP tool result.

    Ditto returns pretty-printed JSON as text content.
    """
    if result is None or not getattr(result, "content", None):
        return None
    for piece in result.content:
        text = getattr(piece, "text", None)
        if text is None:
            continue
        try:
            return json.loads(text)
        except json.JSONDecodeError:
            return text
    return None
