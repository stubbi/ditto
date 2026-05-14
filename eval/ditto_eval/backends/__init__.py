"""Memory backend adapters.

A backend is any system that implements `MemoryBackend`. Concrete adapters:

- `stub.StubBackend` — reference in-memory impl, no semantic search
- (forthcoming) `mem0.Mem0Backend`
- (forthcoming) `zep.ZepBackend`
- (forthcoming) `mastra.MastraBackend`
- (forthcoming) `mempalace.MemPalaceBackend` — via MCP
- (forthcoming) `gbrain.GBrainBackend`     — via MCP
- (forthcoming) `ditto.DittoBackend`       — when the Rust crate exists
"""

from ditto_eval.backends.base import MemoryBackend
from ditto_eval.backends.stub import StubBackend

__all__ = ["MemoryBackend", "StubBackend"]
