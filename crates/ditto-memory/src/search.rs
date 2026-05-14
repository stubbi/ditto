//! Search types: `SearchQuery`, `SearchMode`, `SearchResult`.

use serde::{Deserialize, Serialize};

use ditto_core::{EventId, ScopeId, TenantId};

/// Cost-aware retrieval mode. v0 backends may treat all modes identically.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SearchMode {
    /// BM25 + KG entity exact match. No LLM calls. p50 < 5ms target.
    Cheap,
    /// BM25 + vector + KG, RRF + late-interaction rerank. No LLM on hot path.
    Standard,
    /// Standard + query expansion + cross-encoder rerank + multi-hop KG.
    Deep,
}

impl Default for SearchMode {
    fn default() -> Self {
        Self::Standard
    }
}

#[derive(Clone, Debug)]
pub struct SearchQuery {
    pub query: String,
    pub tenant_id: TenantId,
    pub scope_id: Option<ScopeId>,
    pub sources: Option<Vec<String>>,
    pub k: usize,
    pub mode: SearchMode,
}

impl SearchQuery {
    pub fn new(query: impl Into<String>, tenant_id: TenantId) -> Self {
        Self {
            query: query.into(),
            tenant_id,
            scope_id: None,
            sources: None,
            k: 10,
            mode: SearchMode::default(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SearchResult {
    pub event_id: EventId,
    pub content: String,
    pub score: f32,
    pub source_event_ids: Vec<EventId>,
    pub metadata: serde_json::Value,
}

/// A dense-vector retrieval request. Distinct from `SearchQuery` because the
/// caller has already done the embedding work; storage backends only need to
/// know the vector + filters, not the original text.
#[derive(Clone, Debug)]
pub struct VectorSearchQuery {
    pub embedding: Vec<f32>,
    pub tenant_id: TenantId,
    pub scope_id: Option<ScopeId>,
    pub sources: Option<Vec<String>>,
    pub k: usize,
}

/// Rich search result with retrieval provenance — `why_retrieved`,
/// per-leg ranks, rejected candidates. Returned by `MemoryController::
/// search_explain`. Architecture spec: this is a first-class API, not
/// debug-only.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SearchExplained {
    pub results: Vec<SearchResult>,
    pub mode_used: SearchMode,
    pub embedder_used: bool,
    /// Event ids in BM25-leg order (full leg, before fusion). Empty when
    /// the mode skipped BM25 (no current mode does, but reserved).
    pub bm25_ranks: Vec<EventId>,
    /// Event ids in vector-leg order. Empty when mode is Cheap or no
    /// embedder was configured.
    pub vector_ranks: Vec<EventId>,
    /// Per-result attribution: which leg(s) saw it, fused score, rank in
    /// each leg (1-indexed; `None` if absent from that leg).
    pub why_retrieved: Vec<WhyRetrieved>,
    /// Candidates that appeared in some leg but did not make the top-K.
    /// Sorted by fused score descending; capped to top-32 to keep the
    /// payload bounded.
    pub rejected_candidates: Vec<RejectedCandidate>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WhyRetrieved {
    pub event_id: EventId,
    pub fused_score: f32,
    pub bm25_rank: Option<usize>,
    pub vector_rank: Option<usize>,
    pub bm25_score: Option<f32>,
    pub vector_score: Option<f32>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RejectedCandidate {
    pub event_id: EventId,
    pub fused_score: f32,
    pub bm25_rank: Option<usize>,
    pub vector_rank: Option<usize>,
}
