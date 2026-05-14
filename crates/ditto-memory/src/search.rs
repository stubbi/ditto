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
