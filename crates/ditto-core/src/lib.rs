//! Core types for the Ditto agent memory system.
//!
//! This crate contains the data model, content addressing, canonical JSON encoding,
//! and Ed25519 signing primitives. It has no I/O dependencies — every higher-level
//! crate (storage backends, controller, CLI, MCP server) consumes these types.
//!
//! The contracts encoded here must stay stable across the workspace: changing the
//! canonical JSON encoding or the receipt structure breaks every persisted memory.
//! See [`docs/architecture/memory.md`] in the repo for the v2 commitment.

pub mod canonical;
pub mod error;
pub mod graph;
pub mod id;
pub mod signing;
pub mod types;

pub use error::Error;
pub use graph::{Edge, EdgeId, NewEdge, NewNode, Node, NodeId, SupersedePolicy};
pub use id::{EventId, ScopeId, TenantId};
pub use signing::{InstallKey, Signature, VerifyingKey};
pub use types::{Event, Receipt, SchemaVersion, Slot};

/// The schema version emitted by this crate. Increment when the canonical
/// payload encoding, receipt structure, or signed-bytes layout changes.
pub const CURRENT_SCHEMA_VERSION: SchemaVersion = SchemaVersion(1);
