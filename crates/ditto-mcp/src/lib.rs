//! MCP server transport for Ditto.
//!
//! Exposes the [`MemoryController`] surface as MCP tools over stdio, so any
//! MCP-speaking client (Claude Code, Cursor, Zed, Codex Desktop, …) can read
//! and write Ditto's memory without a separate REST API.
//!
//! Built on the official `rmcp` SDK (`github.com/modelcontextprotocol/rust-sdk`)
//! per the "lean on well-maintained OSS where it earns it" policy. We own the
//! tool surface and its semantics; rmcp owns the JSON-RPC + stdio plumbing.

pub mod server;

pub use server::{serve_stdio, DittoMcpServer};
