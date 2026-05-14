//! The MemoryController + Storage trait.
//!
//! `Storage` is the seam between the controller and the actual database. The
//! v0 implementation exposes the in-process `InMemoryStorage` (for tests and a
//! placeholder for the future SQLite embedded mode); `ditto-storage-postgres`
//! provides the production Postgres backend.
//!
//! The controller's job is to enforce the single-writer invariant: every
//! `write` goes through `commit`, which (a) checks idempotency against the
//! content-addressed event_id, (b) emits a signed receipt, (c) hands off to
//! storage in a single transaction.

pub mod contradiction;
pub mod controller;
pub mod embedder;
pub mod extractor;
pub mod in_memory;
pub mod llm_extractor;
pub mod llm_reranker;
pub mod long_sleep;
pub mod policy;
pub mod reranker;
pub mod search;
pub mod storage;
pub mod working;

pub use contradiction::{
    ContradictionResolver, HeuristicContradictionResolver, NoopContradictionResolver,
};
pub use controller::{
    Authority, ConsolidationMode, ConsolidationReport, DeletionProof, DeletionTarget,
    ImportReport, MemoryController, UpdateError,
};
pub use embedder::{cosine, DeterministicEmbedder, Embedder, EmbedderError, EMBEDDING_DIM};
pub use extractor::{name_to_node_id, Extraction, Extractor, NoopExtractor, ProposedFact, RuleExtractor};
pub use in_memory::InMemoryStorage;
pub use long_sleep::{LongSleepConfig, LongSleepScheduler, LongSleepTick};
#[cfg(feature = "embedders-http")]
pub use llm_extractor::LlmExtractor;
pub use llm_extractor::LlmExtractorError;
#[cfg(feature = "embedders-http")]
pub use llm_reranker::LlmReranker;
pub use llm_reranker::LlmRerankerError;
pub use policy::{HeuristicPolicy, Operation, Policy, PolicyContext, RefusePolicy};
pub use reranker::{NoopReranker, Reranker, ReverseReranker};
pub use search::{
    RejectedCandidate, SearchExplained, SearchMode, SearchQuery, SearchResult, VectorSearchQuery,
    WhyRetrieved,
};
pub use storage::Storage;
pub use working::{Observation, ObservationKind, WorkingMemory};
