//! Late-interaction reranker trait.
//!
//! Single-vector retrieval (BM25 + dense cosine) is the floor; late-
//! interaction rerankers like ColBERT (Khattab & Zaharia 2020),
//! ColBERTv2, JaColBERT, and ColPali consistently top them on
//! out-of-domain queries. The reason: a single 1536-dim vector cannot
//! represent every aspect of a passage, but per-token embeddings scored
//! by max-sim against query tokens can.
//!
//! The integration path:
//! 1. **v0 (this module)**: trait surface + NoopReranker. The controller
//!    calls `reranker.rerank(query, top_k)` after RRF fusion + relevance
//!    gating and before opening the labile window. By default, the noop
//!    impl returns its input untouched — current bench numbers don't
//!    change.
//! 2. **v1**: in-process Rust reranker for the top 50 candidates using
//!    quantized embeddings + a max-sim kernel. The trade-off is latency
//!    for relevance — feasible to keep the hop sub-100ms per query.
//! 3. **v2**: external service (e.g., a ColBERTv2 gRPC endpoint or the
//!    Cohere Rerank API) for tenants that want the highest-quality
//!    reranking without an in-process embedding model.
//!
//! Why a trait and not a hard-coded dependency: the right reranker is
//! tenant-dependent. A latency-sensitive consumer wants the in-process
//! kernel; a research-oriented one wants the external service; a CI
//! pipeline wants the noop so tests are deterministic.

use async_trait::async_trait;

use crate::search::SearchResult;

/// Rerank a list of retrieval candidates. Implementations should preserve
/// each result's `event_id` and `source_event_ids` — only the order (and
/// optionally the `score`) should change. If an implementation drops
/// records it should do so explicitly via `Vec::truncate`, not by
/// silently filtering, so the controller's downstream gates can reason
/// about what's missing.
#[async_trait]
pub trait Reranker: Send + Sync {
    async fn rerank(&self, query: &str, results: Vec<SearchResult>) -> Vec<SearchResult>;
}

/// Default impl. Returns input untouched. Used when no reranker is wired
/// up — keeps the search path's contract identical to pre-rerank builds.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoopReranker;

#[async_trait]
impl Reranker for NoopReranker {
    async fn rerank(&self, _query: &str, results: Vec<SearchResult>) -> Vec<SearchResult> {
        results
    }
}

/// Reverse-order reranker. Useful for tests to assert the controller
/// actually calls the reranker (rather than skipping it) — wire it in,
/// observe that the top result ends up at the bottom, and you've
/// confirmed the integration.
#[derive(Clone, Copy, Debug, Default)]
pub struct ReverseReranker;

#[async_trait]
impl Reranker for ReverseReranker {
    async fn rerank(&self, _query: &str, mut results: Vec<SearchResult>) -> Vec<SearchResult> {
        results.reverse();
        results
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use ditto_core::EventId;

    use super::*;

    fn r(id: u8, score: f32) -> SearchResult {
        SearchResult {
            event_id: EventId([id; 32]),
            content: format!("ev-{id}"),
            score,
            source_event_ids: vec![EventId([id; 32])],
            metadata: json!({}),
        }
    }

    #[tokio::test]
    async fn noop_returns_input_untouched() {
        let input = vec![r(1, 0.9), r(2, 0.5)];
        let out = NoopReranker.rerank("q", input.clone()).await;
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].event_id, input[0].event_id);
        assert_eq!(out[1].event_id, input[1].event_id);
    }

    #[tokio::test]
    async fn reverse_swaps_order() {
        let input = vec![r(1, 0.9), r(2, 0.5), r(3, 0.1)];
        let out = ReverseReranker.rerank("q", input).await;
        assert_eq!(out[0].event_id, EventId([3; 32]));
        assert_eq!(out[2].event_id, EventId([1; 32]));
    }
}
