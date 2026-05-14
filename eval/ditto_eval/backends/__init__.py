"""Memory backend adapters.

A backend is any system that implements `MemoryBackend`. Concrete adapters:

- `stub.StubBackend` — reference in-memory substring scanner; control floor.
- `ditto.DittoBackend` — speaks MCP stdio to a `ditto serve` subprocess.
  This is the production integration path.
- `mem0_backend.Mem0Backend` — adapter for Mem0; requires OPENAI_API_KEY.
  One competitor adapter, kept deliberately bounded. We cite published
  numbers for Zep, Mastra, MemPalace, gbrain instead of building wrappers.
"""

from ditto_eval.backends.base import MemoryBackend
from ditto_eval.backends.ditto import DittoBackend
from ditto_eval.backends.stub import StubBackend

__all__ = ["DittoBackend", "MemoryBackend", "StubBackend"]
