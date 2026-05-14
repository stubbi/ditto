//! NC-doc renderer: projects bi-temporal NC-graph state into per-entity
//! Markdown pages.
//!
//! Three things this crate gives Ditto that Karpathy's LLM Wiki, OpenKB, and
//! Epsilla Semantic Graph do not:
//!
//! 1. **Bi-temporal projection.** The wiki for an entity renders both
//!    current and historical facts directly from `nc_edge`'s
//!    `t_valid`/`t_invalid` columns. No lint pass needed to detect "stale
//!    claims" — they're explicitly invalidated in the graph.
//! 2. **Provenance trail.** Every claim on a page links back to the
//!    episodic events that produced it, via `nc_edge.provenance`.
//! 3. **Deterministic output.** Same NC-graph state → byte-identical
//!    Markdown. Content hashes are checked-in to the manifest so re-renders
//!    diff cleanly.
//!
//! The renderer is one-way. The NC-graph is the source of truth; Markdown
//! files are a projection. Users should not edit pages by hand — changes are
//! overwritten on next render.

pub mod error;
pub mod filesystem;
pub mod job;
pub mod manifest;
pub mod markdown;

pub use error::RenderError;
pub use filesystem::{Filesystem, InMemoryFilesystem, LocalFilesystem};
pub use job::{RenderJob, RenderReport};
pub use manifest::Manifest;
pub use markdown::MarkdownRenderer;
