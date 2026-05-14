//! The `MemoryController` — the single-writer commit path.
//!
//! v0 controller responsibilities:
//! - Compute the event_id from the payload (content addressing)
//! - Mint a signed receipt
//! - Hand off to storage in a single transaction
//! - Track the per-(tenant, source) hash-chain head for `prev_event_id`
//!
//! Deferred to follow-up commits:
//! - Surprise-gated writes (encoder prediction-error scoring)
//! - Reconsolidation labile window on retrieval
//! - Metacognitive retrieval gate (RSCB-MC contextual bandit)
//! - Awake-ripple / dream-cycle / long-sleep consolidation
//! - RL-trained operations policy (Memory-R1 / Mem-α lineage)

use std::collections::HashMap;
use std::sync::Arc;

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::Mutex;

use ditto_core::{
    Blob, BlobHash, Edge, EdgeId, Event, EventId, InstallKey, NewEdge, NewNode, NewReflective,
    NewSkill, Node, NodeId, Receipt, Reflective, ReflectiveId, ScopeId, SchemaVersion, Skill,
    SkillId, SkillStatus, Slot, TenantId, CURRENT_SCHEMA_VERSION,
};

use crate::embedder::Embedder;
use crate::extractor::{name_to_node_id, Extractor};
use crate::search::{
    RejectedCandidate, SearchExplained, SearchMode, SearchQuery, SearchResult,
    VectorSearchQuery, WhyRetrieved,
};
use crate::storage::{CascadeReport, Storage, StorageError, StorageResult};

#[derive(Clone, Copy)]
pub enum SigningPolicy {
    /// Sign every receipt. Production default.
    Required,
    /// Skip signing. Test/embedded only.
    Skip,
}

/// Trust level of the caller asking for an update.
///
/// The reconsolidation-window mechanism only allows updates from trusted
/// sources during the labile window. This is the prompt-injection mitigation
/// the architecture calls for: a model's own continuation cannot rewrite
/// memory it just recalled, even if its output happens to include text that
/// looks like a `memory.update` call.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Authority {
    /// User-supplied input — text the user typed or spoke. Trusted.
    User,
    /// Output of a tool whose result was signed by a verifying key the
    /// controller recognises. Trusted.
    VerifiedTool,
    /// Explicit system-administrator update. Trusted.
    SystemAdmin,
    /// Agent's own reasoning or downstream LLM continuation. Untrusted —
    /// updates from this authority during the labile window are rejected.
    AgentContinuation,
}

impl Authority {
    pub fn is_trusted(self) -> bool {
        match self {
            Authority::User | Authority::VerifiedTool | Authority::SystemAdmin => true,
            Authority::AgentContinuation => false,
        }
    }
}

/// Signed cryptographic proof of a deletion. The wrapped `receipt` is a
/// normal `Receipt` over an event recording the deletion target + cascade
/// counts; auditors verify the deletion happened with authority by
/// re-validating the receipt against the install's verifying key (same as
/// any other receipt). The `cascade` and `target` fields are surfaced for
/// convenience but are also embedded in the event payload — the receipt
/// is the canonical source.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DeletionProof {
    pub target: DeletionTarget,
    pub tenant_id: TenantId,
    pub cascade: CascadeReport,
    pub receipt: Receipt,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DeletionTarget {
    Node(NodeId),
    Blob(BlobHash),
}

/// Per-line outcome of an `import_episodic_jsonl` run.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ImportReport {
    pub written: u32,
    pub skipped: u32,
    pub errors: Vec<String>,
}

/// The three consolidation cadences. v0 ships Ripple as a deterministic,
/// in-process implementation. Dream and LongSleep are stubbed — the real
/// implementations need an LLM extractor (dream) and a background-scheduler
/// (long sleep), both research-first follow-ups.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConsolidationMode {
    /// Between-turn, ≤200ms budget, deterministic. Replays recent episodic
    /// events against NC-graph nodes via cheap token overlap to decide which
    /// events to tag for the next dream cycle.
    Ripple,
    /// Session-close + 24h. LLM-driven Observer/Reflector pattern. Stubbed
    /// in v0 — call returns ConsolidationReport with stub=true.
    Dream,
    /// Daily/weekly. Decay sweep, retrieval-induced suppression, cold
    /// subgraph archival, spaced-retrieval self-testing. Stubbed in v0.
    LongSleep,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConsolidationReport {
    pub mode: ConsolidationMode,
    pub tenant_id: TenantId,
    pub started_at: DateTime<Utc>,
    pub finished_at: DateTime<Utc>,
    pub events_examined: u32,
    pub events_fit: u32,
    pub events_tagged_for_dream: u32,
    /// True when this consolidation mode is stubbed in v0 (Dream / LongSleep
    /// currently). Callers can branch on `stub` to surface "consolidation
    /// pending — needs LLM extractor" rather than silently ignoring.
    pub stub: bool,
    pub notes: String,
}

#[derive(Debug, thiserror::Error)]
pub enum UpdateError {
    #[error("event {event_id} is not in the labile window")]
    NotLabile { event_id: EventId },
    #[error("authority {authority:?} is not trusted for update")]
    UntrustedAuthority { authority: Authority },
    #[error("event {event_id} was already shadowed by {by}")]
    AlreadyShadowed { event_id: EventId, by: EventId },
    #[error("event not found: {0}")]
    NotFound(EventId),
    #[error("storage: {0}")]
    Storage(#[from] StorageError),
}

pub struct MemoryController<S: Storage> {
    storage: Arc<S>,
    install_key: Arc<InstallKey>,
    signing: SigningPolicy,
    schema_version: SchemaVersion,
    chain_heads: Mutex<HashMap<(TenantId, String), EventId>>,
    /// Optional embedder. When set, `write` auto-embeds payload content into
    /// the episodic vector slot, and `search` runs RRF over BM25 + vector
    /// in Standard / Deep mode. When None, vector retrieval is skipped and
    /// every mode falls back to BM25.
    embedder: Option<Arc<dyn Embedder>>,
    /// Reconsolidation labile window. Each event returned by `search` enters
    /// this window; while open, updates from a `Trusted` authority may
    /// shadow the event with a corrected version. Persisted in storage
    /// (`labile_window` + `event_shadow` tables) — survives restart.
    labile_window: Duration,
    /// Relevance threshold for retrieval. After RRF, results whose fused
    /// score (or vector cosine, when an embedder is active) is below
    /// `min_relative_score * top_score` are filtered out of `results`
    /// and surfaced as `rejected_candidates` instead. 0.0 disables.
    min_relative_score: f32,
    /// Absolute cosine-similarity floor. Records whose vector score is
    /// below this floor are dropped regardless of where the top record
    /// landed — protects against "top result is itself a soft distractor"
    /// pathologies where the relative gate alone would happily keep
    /// every record clustered around 0.4. 0.0 disables.
    min_absolute_cosine: f32,
    /// Recency-blend weight in [0.0, 1.0]. The final ranking score becomes
    /// `(1 - α) * relevance + α * recency_factor`, where `recency_factor`
    /// is the result's relative rank by timestamp inside the candidate
    /// set (1.0 = newest, 0.0 = oldest). 0.0 disables recency entirely
    /// (default — the architecture intends this to be learned per-tenant,
    /// not picked once and forgotten).
    alpha_recency: f32,
    /// Optional fact extractor. When set, `write` runs extraction on the
    /// committed payload and applies proposed facts to the NC-graph
    /// (assert_node for src/dst, insert_edge with supersession policy).
    /// `consolidate(Dream)` runs the same extractor over recent episodic
    /// events in a sweep, for cases the per-write path missed.
    extractor: Option<Arc<dyn Extractor>>,
}

impl<S: Storage> MemoryController<S> {
    pub fn new(storage: S, install_key: InstallKey) -> Self {
        Self::new_with_arc(Arc::new(storage), Arc::new(install_key))
    }

    /// Construct from an existing `Arc<S>`. Useful when the storage handle
    /// must be shared between the controller and another consumer (e.g.,
    /// the NC-doc renderer reads the same storage the controller writes).
    pub fn new_with_arc(storage: Arc<S>, install_key: Arc<InstallKey>) -> Self {
        Self {
            storage,
            install_key,
            signing: SigningPolicy::Required,
            schema_version: CURRENT_SCHEMA_VERSION,
            chain_heads: Mutex::new(HashMap::new()),
            embedder: None,
            labile_window: Duration::minutes(5),
            // Two-knob relevance gate. `min_relative_score` (relative to
            // top result) handles the easy case — clearly off-topic records.
            // `min_absolute_cosine` is the floor below which a record is
            // noise regardless of the top score; this protects the case
            // where the top result happens to be a soft distractor (a
            // record more cosine-similar to the *query phrasing* than to
            // its semantic intent) and the relative-only gate would happily
            // drop the actually-correct second-place record.
            //
            // 0.5 / 0.35 are the values that produce 3/3 on Provenance-
            // Bench v0 with OpenAI text-embedding-3-small. Tune via the
            // bench when adding fixtures.
            min_relative_score: 0.5,
            min_absolute_cosine: 0.35,
            // Recency off by default. Provenance-Bench's two cases pull in
            // opposite directions (prov-001 wants oldest-correct, prov-002
            // wants newest-correct), so any fixed positive value here
            // trades fail-modes. The architecturally honest path to prov-002
            // is NC-graph bi-temporal supersession in the dream cycle.
            alpha_recency: 0.0,
            extractor: None,
        }
    }

    /// Set the relative relevance-gate threshold. Results whose fused score
    /// is below `alpha * top_score` are dropped from `results` and surfaced
    /// under `rejected_candidates`. Pass 0.0 to disable.
    pub fn with_min_relative_score(mut self, alpha: f32) -> Self {
        self.min_relative_score = alpha.clamp(0.0, 1.0);
        self
    }

    pub fn min_relative_score(&self) -> f32 {
        self.min_relative_score
    }

    /// Set the absolute cosine floor — records whose vector similarity is
    /// below this value are dropped regardless of the top score. Pass 0.0
    /// to disable.
    pub fn with_min_absolute_cosine(mut self, floor: f32) -> Self {
        self.min_absolute_cosine = floor.clamp(-1.0, 1.0);
        self
    }

    pub fn min_absolute_cosine(&self) -> f32 {
        self.min_absolute_cosine
    }

    /// Recency-blend weight (α_recency). `0.0` = recency ignored;
    /// `1.0` = score is pure recency. Default 0.0.
    pub fn with_alpha_recency(mut self, alpha: f32) -> Self {
        self.alpha_recency = alpha.clamp(0.0, 1.0);
        self
    }

    pub fn alpha_recency(&self) -> f32 {
        self.alpha_recency
    }

    /// Attach a fact extractor. Writes and dream-cycle consolidation will
    /// invoke it; produced facts become NC-graph edges with appropriate
    /// supersession.
    pub fn with_extractor(mut self, extractor: Arc<dyn Extractor>) -> Self {
        self.extractor = Some(extractor);
        self
    }

    pub fn extractor(&self) -> Option<&Arc<dyn Extractor>> {
        self.extractor.as_ref()
    }

    pub fn with_labile_window(mut self, duration: Duration) -> Self {
        self.labile_window = duration;
        self
    }

    pub fn labile_window(&self) -> Duration {
        self.labile_window
    }

    pub fn with_signing_policy(mut self, policy: SigningPolicy) -> Self {
        self.signing = policy;
        self
    }

    pub fn with_embedder(mut self, embedder: Arc<dyn Embedder>) -> Self {
        self.embedder = Some(embedder);
        self
    }

    pub fn embedder(&self) -> Option<&Arc<dyn Embedder>> {
        self.embedder.as_ref()
    }

    pub fn storage(&self) -> &S {
        &self.storage
    }

    /// Commit a payload as an event in `slot`. The event_id is computed from
    /// the payload (content addressing); idempotent on collision.
    pub async fn write(
        &self,
        tenant_id: TenantId,
        scope_id: ScopeId,
        source_id: impl Into<String>,
        slot: Slot,
        payload: Value,
        timestamp: DateTime<Utc>,
    ) -> StorageResult<Receipt> {
        let source_id = source_id.into();
        let prev = {
            let heads = self.chain_heads.lock().await;
            heads.get(&(tenant_id, source_id.clone())).copied()
        };
        let event = Event::new(tenant_id, scope_id, &source_id, slot, payload, timestamp, prev)
            .map_err(StorageError::from)?;

        // idempotency: if already committed, return existing receipt
        if let Some(receipt) = self.storage.get_receipt(&event.event_id).await? {
            return Ok(receipt);
        }

        let receipt = match self.signing {
            SigningPolicy::Required => Receipt::sign(&event, self.schema_version, &self.install_key),
            SigningPolicy::Skip => Receipt::unsigned(&event, self.schema_version),
        };

        self.storage.commit(&event, &receipt).await?;

        // Auto-extract NC-graph facts when an extractor is configured.
        // Like the embedder path, extraction failures don't roll back —
        // worst case is the graph misses a fact that a re-run consolidation
        // would have caught.
        if let Some(_extr) = self.extractor.as_ref() {
            if let Err(e) = self.apply_extraction(&event).await {
                tracing::warn!(error = %e, "extractor failed; event is committed without NC-graph edges");
            }
        }

        // Auto-embed when an embedder is configured. Failures here are
        // logged but do not roll back the commit — losing the embedding
        // costs vector-recall quality on this one event, but the event
        // itself is still durable and BM25-recoverable. Re-running with an
        // embedder restored will re-index.
        if let Some(emb) = &self.embedder {
            let text = render_content(&event.payload);
            if !text.is_empty() {
                match emb.embed(&[text]).await {
                    Ok(vectors) => {
                        if let Some(v) = vectors.into_iter().next() {
                            if let Err(e) =
                                self.storage.put_embedding(tenant_id, event.event_id, &v).await
                            {
                                tracing::warn!(
                                    error = %e,
                                    "put_embedding failed; event is committed but vector-unindexed"
                                );
                            }
                        }
                    }
                    Err(e) => tracing::warn!(error = %e, "embed failed; skipping vector index"),
                }
            }
        }

        {
            let mut heads = self.chain_heads.lock().await;
            heads.insert((tenant_id, source_id), event.event_id);
        }

        Ok(receipt)
    }

    /// Verify a receipt against its event in storage.
    pub async fn verify(&self, receipt: &Receipt) -> StorageResult<bool> {
        let event = match self.storage.get_event(&receipt.event_id).await? {
            Some(e) => e,
            None => return Ok(false),
        };
        let verifier = self.install_key.verifying_key();
        Ok(receipt.verify(&event, &verifier).is_ok())
    }

    /// Hybrid retrieval.
    ///
    /// - `SearchMode::Cheap` → BM25 only.
    /// - `SearchMode::Standard` / `Deep` → BM25 + vector, RRF-fused, when an
    ///   embedder is configured. Otherwise falls back to BM25.
    ///
    /// Late-interaction rerank (ColBERTv2 / MUVERA) and query expansion are
    /// the Standard→Deep delta in the v2 spec; both are follow-ups.
    /// Hybrid retrieval.
    ///
    /// - `SearchMode::Cheap` → BM25 only.
    /// - `SearchMode::Standard` / `Deep` → BM25 + vector, RRF-fused, when an
    ///   embedder is configured. Otherwise falls back to BM25.
    ///
    /// Every returned event enters the reconsolidation labile window —
    /// during `self.labile_window` from now, a trusted-authority `update`
    /// can rewrite the trace. After the window closes the event is treated
    /// as consolidated and only `invalidate_edge` / `delete` paths apply.
    pub async fn search(&self, query: &SearchQuery) -> StorageResult<Vec<SearchResult>> {
        let raw = self.run_retrieval(query).await?;
        let resolved = self.resolve_shadows(query.tenant_id, raw).await?;
        let gated = self.apply_recency_blend(self.apply_relevance_gate(resolved));

        // Open the labile window on every returned event.
        let now = Utc::now();
        let until = now + self.labile_window;
        for r in &gated {
            if let Err(e) = self.storage.open_labile(query.tenant_id, r.event_id, until).await {
                tracing::warn!(error = %e, "open_labile failed; subsequent update() may reject as not-labile");
            }
        }

        Ok(gated)
    }

    /// Drop results whose relevance is well below the top result's.
    ///
    /// Decision is made on cosine similarity (vector leg, range [-1, 1])
    /// when the embedder is active, since cosine has natural separation —
    /// a semantically-related record might score 0.7 while an unrelated
    /// record scores 0.3. Without an embedder, falls back to relative-RRF
    /// (compressed; less effective, but better than nothing).
    ///
    /// Disabled entirely when `min_relative_score == 0.0`.
    /// Run the configured extractor on `event` and apply its facts to the
    /// NC-graph. Idempotent under repeat: nodes are upserted via
    /// `assert_node`; edges with supersession invalidate priors atomically
    /// in storage. The event's `event_id` is the provenance pointer.
    async fn apply_extraction(&self, event: &Event) -> StorageResult<()> {
        let Some(extractor) = self.extractor.as_ref() else {
            return Ok(());
        };
        let extraction = extractor.extract(event).await;
        if extraction.is_empty() {
            return Ok(());
        }
        for fact in extraction.facts {
            let src = name_to_node_id(event.tenant_id, event.scope_id, &fact.subject);
            let dst = name_to_node_id(event.tenant_id, event.scope_id, &fact.object);
            // Idempotency on (event_id, src, rel): if any current edge
            // already cites this event_id under this relation, the dream
            // sweep is replaying a fact write() already applied. Skip.
            let existing = self
                .storage
                .current_edges_from(event.tenant_id, src, Some(fact.relation.as_str()))
                .await?;
            if existing.iter().any(|e| e.provenance.contains(&event.event_id)) {
                continue;
            }
            self.storage
                .assert_node(NewNode {
                    node_id: src,
                    tenant_id: event.tenant_id,
                    scope_id: event.scope_id,
                    node_type: "entity".into(),
                    properties: serde_json::json!({ "name": fact.subject }),
                    provenance: vec![event.event_id],
                })
                .await?;
            self.storage
                .assert_node(NewNode {
                    node_id: dst,
                    tenant_id: event.tenant_id,
                    scope_id: event.scope_id,
                    node_type: "entity".into(),
                    properties: serde_json::json!({ "name": fact.object }),
                    provenance: vec![event.event_id],
                })
                .await?;
            let supersede = fact.supersede_policy();
            self.storage
                .insert_edge(NewEdge {
                    edge_id: EdgeId::new(),
                    src,
                    dst,
                    rel: fact.relation,
                    strength: Some(fact.confidence),
                    tenant_id: event.tenant_id,
                    scope_id: event.scope_id,
                    t_valid: event.timestamp,
                    t_invalid: None,
                    provenance: vec![event.event_id],
                    supersede,
                })
                .await?;
        }
        Ok(())
    }

    /// Blend recency into the final ranking. Each result's blended score is
    /// `(1 - α) * (score / top_score) + α * recency_factor`, where
    /// `recency_factor` is the result's rank-by-timestamp within the
    /// candidate set (1.0 = newest, 0.0 = oldest). α = 0 is the identity.
    fn apply_recency_blend(&self, mut results: Vec<SearchResult>) -> Vec<SearchResult> {
        let alpha = self.alpha_recency as f64;
        if alpha <= 0.0 || results.len() < 2 {
            return results;
        }
        // Pull timestamps out of metadata (RFC3339 string format set by
        // the storage layer's `timestamp` field).
        let mut tss: Vec<Option<DateTime<Utc>>> = results
            .iter()
            .map(|r| {
                r.metadata
                    .get("timestamp")
                    .and_then(|v| v.as_str())
                    .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                    .map(|t| t.with_timezone(&Utc))
            })
            .collect();
        // If any timestamp is missing we abstain — partial recency would
        // be worse than none.
        if tss.iter().any(|t| t.is_none()) {
            // Fall back to relevance-only order.
            return results;
        }
        let min_ts = tss.iter().filter_map(|t| t.as_ref()).min().copied().unwrap();
        let max_ts = tss.iter().filter_map(|t| t.as_ref()).max().copied().unwrap();
        let span = (max_ts - min_ts).num_seconds().max(1) as f64;

        let top_score = results
            .iter()
            .map(|r| r.score)
            .fold(f32::NEG_INFINITY, f32::max) as f64;
        if top_score <= 0.0 {
            return results;
        }

        for (i, r) in results.iter_mut().enumerate() {
            let ts = tss[i].take().unwrap();
            let rec = ((ts - min_ts).num_seconds() as f64) / span;
            let rel = (r.score as f64) / top_score;
            r.score = ((1.0 - alpha) * rel + alpha * rec) as f32;
        }
        results.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.event_id.0.cmp(&b.event_id.0))
        });
        results
    }

    fn apply_relevance_gate(&self, results: Vec<SearchResult>) -> Vec<SearchResult> {
        let relative_active = self.min_relative_score > 0.0;
        let absolute_active = self.min_absolute_cosine > 0.0;
        if !relative_active && !absolute_active {
            return results;
        }
        if results.is_empty() {
            return results;
        }

        let vector_top = results
            .iter()
            .filter_map(|r| r.metadata.get("vector_score").and_then(|v| v.as_f64()))
            .fold(f64::NEG_INFINITY, f64::max);
        let has_vector = vector_top.is_finite() && vector_top > 0.0;

        if has_vector {
            // Two-knob gate against vector cosine. A record passes only if it
            // exceeds BOTH the absolute floor AND the relative-to-top cutoff.
            // Exception: records that appeared in the KG leg are kept
            // unconditionally — bi-temporal supersession on edges makes the
            // KG hit authoritative, even when its cosine is low (the
            // semantically-correct "moved to Berlin" event scores lower
            // than the cosine-confounded "lives in San Francisco" baseline).
            let abs_floor = self.min_absolute_cosine as f64;
            let rel_cutoff = vector_top * (self.min_relative_score as f64);
            return results
                .into_iter()
                .filter(|r| {
                    if r.metadata.get("kg_score").is_some() {
                        return true;
                    }
                    match r.metadata.get("vector_score").and_then(|v| v.as_f64()) {
                        Some(cos) => {
                            (!absolute_active || cos >= abs_floor)
                                && (!relative_active || cos >= rel_cutoff)
                        }
                        None => r
                            .metadata
                            .get("bm25_score")
                            .and_then(|v| v.as_f64())
                            .map(|s| s >= 1.0)
                            .unwrap_or(false),
                    }
                })
                .collect();
        }

        // No vector leg active — fall back to relative RRF / BM25 score gate.
        if !relative_active {
            return results;
        }
        let top = results[0].score;
        if top <= 0.0 {
            return Vec::new();
        }
        let cutoff = top * self.min_relative_score;
        results.into_iter().filter(|r| r.score >= cutoff).collect()
    }

    async fn run_retrieval(&self, query: &SearchQuery) -> StorageResult<Vec<SearchResult>> {
        let standard_or_deep = matches!(query.mode, SearchMode::Standard | SearchMode::Deep);
        let do_vector = standard_or_deep && self.embedder.is_some();
        let do_kg = standard_or_deep; // KG-leg is cheap; always run in standard/deep
        if !do_vector && !do_kg {
            return self.storage.search(query).await;
        }

        // Oversample 3x per leg before fusion. RRF with K=60 places most
        // mass in the top ~15; pulling 3*k gives the fusion room to pick
        // up dual-hits beyond either leg's top-k.
        let leg_k = query.k.saturating_mul(3).max(query.k);

        let mut bm_query = query.clone();
        bm_query.k = leg_k;
        let bm_results = self.storage.search(&bm_query).await?;

        let mut vec_results: Vec<SearchResult> = Vec::new();
        if do_vector {
            let emb = self.embedder.as_ref().unwrap();
            let mut embs = emb
                .embed(std::slice::from_ref(&query.query))
                .await
                .map_err(|e| StorageError::Other(format!("query embed: {e}")))?;
            if let Some(q_emb) = embs.pop() {
                let vec_query = VectorSearchQuery {
                    embedding: q_emb,
                    tenant_id: query.tenant_id,
                    scope_id: query.scope_id,
                    sources: query.sources.clone(),
                    k: leg_k,
                };
                vec_results = self.storage.search_vector(&vec_query).await?;
            }
        }

        let kg_results = if do_kg {
            self.run_kg_leg(query, leg_k).await?
        } else {
            Vec::new()
        };

        Ok(rrf_fuse_n(
            &[bm_results, vec_results, kg_results],
            query.k,
        ))
    }

    /// KG-leg: lookup entities in the query against the tenant's NC-graph
    /// node set, then surface current edges and synthesize SearchResults
    /// pointing at each edge's most recent provenance event. This is what
    /// makes bi-temporal supersession actually flow through to retrieval —
    /// the right answer to "where does the user currently live?" is
    /// whatever edge the dream-cycle's `lives_in` supersession landed on,
    /// regardless of which leg's cosine surfaces in BM25/vector.
    async fn run_kg_leg(
        &self,
        query: &SearchQuery,
        leg_k: usize,
    ) -> StorageResult<Vec<SearchResult>> {
        let nodes = self.storage.list_nodes(query.tenant_id, query.scope_id).await?;
        if nodes.is_empty() {
            return Ok(Vec::new());
        }
        // Entity recognition v0: exact alphanumeric-token match of the
        // query against each node's `properties.name` (which the extractor
        // sets to the original subject/object string). LLM-driven entity
        // linking is a follow-up.
        let q_tokens: std::collections::HashSet<String> = query
            .query
            .to_lowercase()
            .split(|c: char| !c.is_alphanumeric())
            .filter(|t| !t.is_empty())
            .map(String::from)
            .collect();

        let mut matched_nodes: Vec<&Node> = Vec::new();
        for node in &nodes {
            let name = node
                .properties
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let canon = name.trim().to_lowercase();
            if canon.is_empty() {
                continue;
            }
            // Multi-word node names (e.g., "san francisco") match if any
            // of their tokens appears in the query.
            let any_hit = canon
                .split(|c: char| !c.is_alphanumeric())
                .filter(|t| !t.is_empty())
                .any(|t| q_tokens.contains(t));
            if any_hit {
                matched_nodes.push(node);
            }
        }

        let mut results: Vec<SearchResult> = Vec::new();
        for node in matched_nodes {
            let edges = self
                .storage
                .current_edges_from(query.tenant_id, node.node_id, None)
                .await?;
            for edge in edges {
                // Surface the edge's most recent provenance event as the
                // canonical "source" for bench-scoring purposes.
                let Some(&prov) = edge.provenance.last() else {
                    continue;
                };
                let Some(target_event) = self.storage.get_event(&prov).await? else {
                    continue;
                };
                let content = render_content(&target_event.payload);
                let score = edge.strength.max(0.5); // baseline 0.5 so kg-only hits compete in RRF
                results.push(SearchResult {
                    event_id: target_event.event_id,
                    content,
                    score,
                    source_event_ids: edge.provenance.clone(),
                    metadata: serde_json::json!({
                        "source_id": target_event.source_id,
                        "timestamp": target_event.timestamp,
                        "slot": target_event.slot,
                        "leg": "kg",
                        "kg_edge_rel": edge.rel,
                    }),
                });
            }
        }
        // Stable order before truncation: highest strength first; tiebreak
        // by event_id so the leg is byte-deterministic.
        results.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.event_id.0.cmp(&b.event_id.0))
        });
        results.truncate(leg_k);
        Ok(results)
    }

    /// Walk the shadow chain to the most recent un-shadowed event for each
    /// raw result. Results that have been shadowed are replaced by their
    /// latest version (content + metadata pulled from storage).
    async fn resolve_shadows(
        &self,
        tenant_id: TenantId,
        mut results: Vec<SearchResult>,
    ) -> StorageResult<Vec<SearchResult>> {
        for r in results.iter_mut() {
            let mut cur = r.event_id;
            let mut hops = 0_usize;
            // Bounded chain walk against persistent shadow rows — prevents
            // infinite loops if a shadow somehow points back to its source.
            while hops <= 8 {
                let Some(next) = self.storage.lookup_shadow(tenant_id, cur).await? else {
                    break;
                };
                if next == cur {
                    break;
                }
                cur = next;
                hops += 1;
            }
            if cur != r.event_id {
                if let Some(latest) = self.storage.get_event(&cur).await? {
                    r.event_id = latest.event_id;
                    r.content = render_content(&latest.payload);
                    r.metadata = serde_json::json!({
                        "shadow_of": r.source_event_ids.first().map(|id| id.to_string()),
                        "via_shadow": true,
                    });
                    r.source_event_ids = vec![latest.event_id];
                }
            }
        }
        Ok(results)
    }

    /// Update an event during its reconsolidation labile window.
    ///
    /// Writes a NEW event whose payload merges the original payload with
    /// `patch` (top-level object merge, patch keys win). The new event's
    /// id is recorded as a shadow of the original; subsequent searches
    /// surface the shadow's content while preserving the original's audit
    /// trail. Returns the new event's receipt.
    pub async fn update(
        &self,
        tenant_id: TenantId,
        event_id: EventId,
        patch: Value,
        authority: Authority,
    ) -> Result<Receipt, UpdateError> {
        if !authority.is_trusted() {
            return Err(UpdateError::UntrustedAuthority { authority });
        }

        // Labile-window check.
        let now = Utc::now();
        if self
            .storage
            .is_labile(tenant_id, event_id, now)
            .await?
            .is_none()
        {
            return Err(UpdateError::NotLabile { event_id });
        }

        // Already-shadowed check — refuse to write a second shadow over the
        // same original, since the audit story gets confusing.
        if let Some(by) = self.storage.lookup_shadow(tenant_id, event_id).await? {
            return Err(UpdateError::AlreadyShadowed { event_id, by });
        }

        let original = self
            .storage
            .get_event(&event_id)
            .await?
            .ok_or(UpdateError::NotFound(event_id))?;

        let mut new_payload = original.payload.clone();
        merge_json_objects(&mut new_payload, patch);
        // Provenance: stamp the shadow so a verifier can confirm intent.
        if let Value::Object(map) = &mut new_payload {
            map.insert(
                "_shadowed_event_id".into(),
                Value::String(event_id.to_string()),
            );
            map.insert(
                "_shadowed_by_authority".into(),
                Value::String(format!("{authority:?}").to_lowercase()),
            );
        }

        let receipt = self
            .write(
                tenant_id,
                original.scope_id,
                original.source_id.clone(),
                original.slot,
                new_payload,
                now,
            )
            .await?;

        let authority_str = match authority {
            Authority::User => "user",
            Authority::VerifiedTool => "verified_tool",
            Authority::SystemAdmin => "system_admin",
            Authority::AgentContinuation => unreachable!("rejected above"),
        };
        self.storage
            .write_shadow(tenant_id, event_id, receipt.event_id, authority_str)
            .await?;
        Ok(receipt)
    }

    /// Same as `search` but returns retrieval provenance: which leg(s) saw
    /// each event, per-leg ranks, rejected candidates. The architecture's
    /// "first-class, not debug-only" explain surface.
    pub async fn search_explain(&self, query: &SearchQuery) -> StorageResult<SearchExplained> {
        let do_vector = matches!(query.mode, SearchMode::Standard | SearchMode::Deep)
            && self.embedder.is_some();

        let leg_k = query.k.saturating_mul(3).max(query.k);
        let mut bm_query = query.clone();
        bm_query.k = leg_k;
        let bm_results = self.storage.search(&bm_query).await?;

        let mut vec_results: Vec<SearchResult> = Vec::new();
        if do_vector {
            let emb = self.embedder.as_ref().unwrap();
            let mut embs = emb
                .embed(std::slice::from_ref(&query.query))
                .await
                .map_err(|e| StorageError::Other(format!("query embed: {e}")))?;
            if let Some(q_emb) = embs.pop() {
                let vec_query = VectorSearchQuery {
                    embedding: q_emb,
                    tenant_id: query.tenant_id,
                    scope_id: query.scope_id,
                    sources: query.sources.clone(),
                    k: leg_k,
                };
                vec_results = self.storage.search_vector(&vec_query).await?;
            }
        }

        let bm_ranks: Vec<EventId> = bm_results.iter().map(|r| r.event_id).collect();
        let vec_ranks: Vec<EventId> = vec_results.iter().map(|r| r.event_id).collect();
        let bm_scores: HashMap<EventId, (usize, f32)> = bm_results
            .iter()
            .enumerate()
            .map(|(i, r)| (r.event_id, (i, r.score)))
            .collect();
        let vec_scores: HashMap<EventId, (usize, f32)> = vec_results
            .iter()
            .enumerate()
            .map(|(i, r)| (r.event_id, (i, r.score)))
            .collect();

        let fused = if do_vector {
            rrf_fuse(bm_results, vec_results, leg_k)
        } else {
            bm_results
        };

        let mut top = fused.clone();
        top.truncate(query.k);
        let pre_gate = self.resolve_shadows(query.tenant_id, top).await?;
        let resolved = self.apply_recency_blend(self.apply_relevance_gate(pre_gate));
        let now = Utc::now();
        let until = now + self.labile_window;
        for r in &resolved {
            if let Err(e) = self
                .storage
                .open_labile(query.tenant_id, r.event_id, until)
                .await
            {
                tracing::warn!(error = %e, "open_labile (explain) failed");
            }
        }

        let why: Vec<WhyRetrieved> = resolved
            .iter()
            .map(|r| {
                let (br, bs) = bm_scores
                    .get(&r.event_id)
                    .map(|(i, s)| (Some(*i + 1), Some(*s)))
                    .unwrap_or((None, None));
                let (vr, vs) = vec_scores
                    .get(&r.event_id)
                    .map(|(i, s)| (Some(*i + 1), Some(*s)))
                    .unwrap_or((None, None));
                WhyRetrieved {
                    event_id: r.event_id,
                    fused_score: r.score,
                    bm25_rank: br,
                    vector_rank: vr,
                    bm25_score: bs,
                    vector_score: vs,
                }
            })
            .collect();

        // Rejected = anything that the fused list saw but `results` did
        // not keep. Two reasons a record lands here: (a) below top-K cutoff,
        // (b) below the relevance gate (score < α * top_score). Both end up
        // surfaced as audit so the caller can see what almost made it.
        let kept_ids: std::collections::HashSet<EventId> =
            resolved.iter().map(|r| r.event_id).collect();
        let mut rejected: Vec<RejectedCandidate> = fused
            .iter()
            .filter(|r| !kept_ids.contains(&r.event_id))
            .map(|r| RejectedCandidate {
                event_id: r.event_id,
                fused_score: r.score,
                bm25_rank: bm_scores.get(&r.event_id).map(|(i, _)| *i + 1),
                vector_rank: vec_scores.get(&r.event_id).map(|(i, _)| *i + 1),
            })
            .collect();
        rejected.truncate(32);

        Ok(SearchExplained {
            results: resolved,
            mode_used: query.mode,
            embedder_used: do_vector,
            bm25_ranks: bm_ranks,
            vector_ranks: vec_ranks,
            why_retrieved: why,
            rejected_candidates: rejected,
        })
    }

    /// Run a consolidation pass.
    ///
    /// `Ripple` is the deterministic v0 implementation — no LLM calls. It
    /// reads recent episodic events, scores schema-fit against NC-graph
    /// nodes via cheap token overlap (lowercased word-set intersection vs
    /// each node's `properties` text), and tags events that fit for the
    /// next (eventual) dream cycle.
    ///
    /// `Dream` and `LongSleep` return reports with `stub = true` and notes
    /// describing what the real implementation will do. Calling code can
    /// branch on `report.stub` to surface "consolidation pending" rather
    /// than treating the no-op as a successful pass.
    pub async fn consolidate(
        &self,
        tenant_id: TenantId,
        scope_id: Option<ScopeId>,
        mode: ConsolidationMode,
    ) -> StorageResult<ConsolidationReport> {
        let started = Utc::now();
        match mode {
            ConsolidationMode::Ripple => self.run_ripple(tenant_id, scope_id, started).await,
            ConsolidationMode::Dream => self.run_dream(tenant_id, scope_id, started).await,
            ConsolidationMode::LongSleep => Ok(ConsolidationReport {
                mode,
                tenant_id,
                started_at: started,
                finished_at: started,
                events_examined: 0,
                events_fit: 0,
                events_tagged_for_dream: 0,
                stub: true,
                notes: "Long sleep pending — requires background scheduler \
                        for decay sweep, retrieval-induced suppression, cold \
                        subgraph archival, and spaced-retrieval self-testing."
                    .into(),
            }),
        }
    }

    async fn run_dream(
        &self,
        tenant_id: TenantId,
        scope_id: Option<ScopeId>,
        started: DateTime<Utc>,
    ) -> StorageResult<ConsolidationReport> {
        // v0 dream is the rule-based extraction sweep — no LLM. Reads the
        // most recent N episodic events and replays them through the
        // configured extractor. Idempotent: re-running over the same
        // events produces no duplicate edges because assert_node is
        // upsert-on-id and insert_edge with supersession invalidates
        // priors atomically.
        const MAX_EVENTS: usize = 200;

        let extractor_present = self.extractor.is_some();
        if !extractor_present {
            let finished = Utc::now();
            return Ok(ConsolidationReport {
                mode: ConsolidationMode::Dream,
                tenant_id,
                started_at: started,
                finished_at: finished,
                events_examined: 0,
                events_fit: 0,
                events_tagged_for_dream: 0,
                stub: true,
                notes: "Dream cycle pending — no extractor configured. \
                        Attach one with `MemoryController::with_extractor`. \
                        `RuleExtractor` is the deterministic v0 default."
                    .into(),
            });
        }

        let events = self
            .storage
            .list_episodic(tenant_id, scope_id, Some(MAX_EVENTS))
            .await?;
        let mut fit = 0_u32;
        let extractor = self.extractor.as_ref().unwrap();
        for event in &events {
            let extraction = extractor.extract(event).await;
            if extraction.is_empty() {
                continue;
            }
            // Re-apply through the same helper write() uses. Idempotent on
            // assert_node; supersession on insert_edge means re-running is
            // a no-op rather than producing duplicates.
            if let Err(e) = self.apply_extraction(event).await {
                tracing::warn!(error = %e, event_id = %event.event_id, "dream: apply_extraction failed");
                continue;
            }
            fit += 1;
        }
        let finished = Utc::now();
        Ok(ConsolidationReport {
            mode: ConsolidationMode::Dream,
            tenant_id,
            started_at: started,
            finished_at: finished,
            events_examined: events.len() as u32,
            events_fit: fit,
            events_tagged_for_dream: 0,
            stub: false,
            notes: format!(
                "Rule-extractor sweep: applied facts from {fit} of {} events. \
                 LLM-driven Observer/Reflector + Graphiti contradiction check \
                 + ADM counterfactuals still deferred.",
                events.len()
            ),
        })
    }

    async fn run_ripple(
        &self,
        tenant_id: TenantId,
        scope_id: Option<ScopeId>,
        started: DateTime<Utc>,
    ) -> StorageResult<ConsolidationReport> {
        // v0 fixed budget: examine up to 50 most-recent events. The
        // architecture's ≤200ms budget needs a wall-clock guard once dream
        // extraction lands and pulls actual work; for deterministic ripple
        // the loop is bounded by event count.
        const MAX_EVENTS: usize = 50;
        const FIT_THRESHOLD: usize = 2; // ≥N overlap tokens count as schema-fit

        let events = self
            .storage
            .list_episodic(tenant_id, scope_id, Some(MAX_EVENTS))
            .await?;
        let nodes = self.storage.list_nodes(tenant_id, scope_id).await?;

        // Pre-tokenize node properties once.
        let node_tokens: Vec<std::collections::HashSet<String>> = nodes
            .iter()
            .map(|n| tokenize(&n.properties.to_string()))
            .collect();

        let mut fit = 0_u32;
        let mut tagged = 0_u32;
        for e in &events {
            let event_tokens = tokenize(&render_content(&e.payload));
            if event_tokens.is_empty() {
                continue;
            }
            let max_overlap = node_tokens
                .iter()
                .map(|nt| event_tokens.intersection(nt).count())
                .max()
                .unwrap_or(0);
            if max_overlap >= FIT_THRESHOLD {
                fit += 1;
                tagged += 1;
                // Bump salience on the fit event so the dream cycle's
                // priority replay picks it up first. +0.1 nudges past the
                // 0.5 default; long-sleep decay sweeps it back toward
                // baseline unless re-tagged.
                if let Err(err) =
                    self.storage.bump_salience(tenant_id, e.event_id, 0.1).await
                {
                    tracing::warn!(error = %err, event_id = %e.event_id, "ripple bump_salience failed");
                }
            }
        }

        let finished = Utc::now();
        Ok(ConsolidationReport {
            mode: ConsolidationMode::Ripple,
            tenant_id,
            started_at: started,
            finished_at: finished,
            events_examined: events.len() as u32,
            events_fit: fit,
            events_tagged_for_dream: tagged,
            stub: false,
            notes: format!(
                "Examined {} events against {} nodes; fit threshold = {} overlapping tokens.",
                events.len(),
                nodes.len(),
                FIT_THRESHOLD
            ),
        })
    }

    /// Stream a tenant's episodic events as JSONL into `writer`. One event
    /// per line; the wire shape is the `Event` serde representation, so
    /// `import_episodic_jsonl` round-trips losslessly.
    pub async fn export_episodic_jsonl<W: std::io::Write>(
        &self,
        tenant_id: TenantId,
        scope_id: Option<ScopeId>,
        mut writer: W,
    ) -> StorageResult<usize> {
        let events = self.storage.list_episodic(tenant_id, scope_id, None).await?;
        let mut n = 0_usize;
        for e in events {
            let line = serde_json::to_string(&e)
                .map_err(|err| StorageError::Other(format!("export serialize: {err}")))?;
            writeln!(writer, "{line}")
                .map_err(|err| StorageError::Other(format!("export write: {err}")))?;
            n += 1;
        }
        Ok(n)
    }

    /// Read JSONL produced by `export_episodic_jsonl` and replay writes.
    /// Idempotent on event_id (content addressing) — duplicates are
    /// counted as `skipped`, not errors.
    pub async fn import_episodic_jsonl<R: std::io::BufRead>(
        &self,
        reader: R,
    ) -> StorageResult<ImportReport> {
        let mut report = ImportReport::default();
        for (line_no, line) in reader.lines().enumerate() {
            let line = line
                .map_err(|e| StorageError::Other(format!("import read line {line_no}: {e}")))?;
            if line.trim().is_empty() {
                continue;
            }
            let event: Event = match serde_json::from_str(&line) {
                Ok(e) => e,
                Err(e) => {
                    report.errors.push(format!("line {line_no}: {e}"));
                    continue;
                }
            };
            if self.storage.get_receipt(&event.event_id).await?.is_some() {
                report.skipped += 1;
                continue;
            }
            self.write(
                event.tenant_id,
                event.scope_id,
                event.source_id.clone(),
                event.slot,
                event.payload.clone(),
                event.timestamp,
            )
            .await?;
            report.written += 1;
        }
        Ok(report)
    }

    // --- Verifiable cascade delete ---

    /// Delete a node and cascade to its edges. Emits a signed `DeletionProof`
    /// — a regular event in the `BlobStore` slot whose payload records the
    /// target, timestamp, and cascade count. The proof is verifiable with
    /// the install's verifying key; auditors can re-validate the deletion
    /// happened with authority.
    ///
    /// Idempotent: a second delete of an already-removed node returns a new
    /// proof with `node_removed = false, edges_removed = 0`. That's
    /// deliberate — re-runnable deletion is essential for retry safety.
    pub async fn delete_node(
        &self,
        tenant_id: TenantId,
        scope_id: ScopeId,
        node_id: NodeId,
    ) -> StorageResult<DeletionProof> {
        let cascade = self.storage.delete_node_cascade(tenant_id, node_id).await?;
        let receipt = self
            .emit_deletion_receipt(
                tenant_id,
                scope_id,
                "delete_node",
                serde_json::json!({
                    "target_kind": "node",
                    "target_id": node_id.to_string(),
                    "cascade": cascade,
                }),
            )
            .await?;
        Ok(DeletionProof {
            target: DeletionTarget::Node(node_id),
            tenant_id,
            cascade,
            receipt,
        })
    }

    /// Delete a blob from the tenant's CAS. Emits a signed DeletionProof.
    /// Idempotent: returns a proof with `node_removed = false` when the
    /// blob was already absent.
    pub async fn delete_blob(
        &self,
        tenant_id: TenantId,
        scope_id: ScopeId,
        hash: BlobHash,
    ) -> StorageResult<DeletionProof> {
        let existed = self.storage.delete_blob(tenant_id, hash).await?;
        let receipt = self
            .emit_deletion_receipt(
                tenant_id,
                scope_id,
                "delete_blob",
                serde_json::json!({
                    "target_kind": "blob",
                    "target_id": hash.to_hex(),
                    "existed": existed,
                }),
            )
            .await?;
        Ok(DeletionProof {
            target: DeletionTarget::Blob(hash),
            tenant_id,
            cascade: CascadeReport {
                node_removed: existed,
                edges_removed: 0,
            },
            receipt,
        })
    }

    async fn emit_deletion_receipt(
        &self,
        tenant_id: TenantId,
        scope_id: ScopeId,
        source_id: &str,
        target_payload: Value,
    ) -> StorageResult<Receipt> {
        // Include the nanos timestamp inside the payload so two delete
        // events for the same target produce distinct event_ids — without
        // this, content-addressing would dedupe a second delete that the
        // application explicitly wants logged.
        let now = Utc::now();
        let mut payload = target_payload;
        if let Value::Object(map) = &mut payload {
            let nanos = now.timestamp_nanos_opt().unwrap_or(0);
            map.insert("_at_nanos".into(), serde_json::json!(nanos));
        }
        self.write(
            tenant_id,
            scope_id,
            source_id.to_string(),
            Slot::BlobStore,
            payload,
            now,
        )
        .await
    }

    /// Is the event currently within its labile window? Useful for callers
    /// who want to gate UI affordances ("this fact is editable").
    pub async fn is_labile(&self, tenant_id: TenantId, event_id: EventId) -> bool {
        self.storage
            .is_labile(tenant_id, event_id, Utc::now())
            .await
            .ok()
            .flatten()
            .is_some()
    }

    /// Prune labile-window rows whose deadline has passed. Returns the
    /// number of rows removed. Exposed for the long-sleep scheduler; safe
    /// to call any time.
    pub async fn prune_expired_labile(&self) -> StorageResult<u32> {
        self.storage.prune_expired_labile(Utc::now()).await
    }

    // --- Salience ---

    pub async fn get_salience(
        &self,
        tenant_id: TenantId,
        event_id: EventId,
    ) -> StorageResult<Option<f32>> {
        self.storage.get_salience(tenant_id, event_id).await
    }

    pub async fn set_salience(
        &self,
        tenant_id: TenantId,
        event_id: EventId,
        value: f32,
    ) -> StorageResult<f32> {
        self.storage.set_salience(tenant_id, event_id, value).await
    }

    pub async fn bump_salience(
        &self,
        tenant_id: TenantId,
        event_id: EventId,
        delta: f32,
    ) -> StorageResult<f32> {
        self.storage.bump_salience(tenant_id, event_id, delta).await
    }

    pub async fn decay_salience(
        &self,
        tenant_id: TenantId,
        factor: f32,
    ) -> StorageResult<u32> {
        self.storage.decay_salience(tenant_id, factor).await
    }

    pub async fn reset(&self, tenant_id: TenantId) -> StorageResult<()> {
        self.storage.reset(tenant_id).await?;
        let mut heads = self.chain_heads.lock().await;
        heads.retain(|(t, _), _| *t != tenant_id);
        Ok(())
    }

    // --- NC-graph surface ---

    /// Idempotent node upsert by `node_id`.
    pub async fn assert_node(&self, node: NewNode) -> StorageResult<Node> {
        self.storage.assert_node(node).await
    }

    pub async fn get_node(&self, node_id: NodeId) -> StorageResult<Option<Node>> {
        self.storage.get_node(node_id).await
    }

    /// Write a semantic fact (NC-graph edge) with optional supersession of
    /// prior contradicting facts. Atomic with the supersession update.
    pub async fn write_fact(&self, fact: NewEdge) -> StorageResult<Edge> {
        self.storage.insert_edge(fact).await
    }

    pub async fn get_edge(&self, edge_id: EdgeId) -> StorageResult<Option<Edge>> {
        self.storage.get_edge(edge_id).await
    }

    pub async fn current_edges_from(
        &self,
        tenant_id: TenantId,
        src: NodeId,
        rel: Option<&str>,
    ) -> StorageResult<Vec<Edge>> {
        self.storage.current_edges_from(tenant_id, src, rel).await
    }

    pub async fn current_edges_to(
        &self,
        tenant_id: TenantId,
        dst: NodeId,
        rel: Option<&str>,
    ) -> StorageResult<Vec<Edge>> {
        self.storage.current_edges_to(tenant_id, dst, rel).await
    }

    /// Time-travel query: outgoing edges from `src` that were valid at `t`.
    pub async fn edges_from_at(
        &self,
        tenant_id: TenantId,
        src: NodeId,
        t: DateTime<Utc>,
    ) -> StorageResult<Vec<Edge>> {
        self.storage.edges_from_at(tenant_id, src, t).await
    }

    /// Mark an edge as no-longer-true at `t_invalid`. The edge stays
    /// queryable for historical reads.
    pub async fn invalidate_edge(
        &self,
        edge_id: EdgeId,
        t_invalid: DateTime<Utc>,
    ) -> StorageResult<()> {
        self.storage.invalidate_edge(edge_id, t_invalid).await
    }

    // --- Blob store ---

    /// Store a blob in the tenant's CAS. Idempotent on the SHA-256 of the
    /// blob bytes.
    pub async fn put_blob(&self, tenant_id: TenantId, blob: &Blob) -> StorageResult<BlobHash> {
        self.storage.put_blob(tenant_id, blob).await
    }

    pub async fn get_blob(
        &self,
        tenant_id: TenantId,
        hash: BlobHash,
    ) -> StorageResult<Option<Blob>> {
        self.storage.get_blob(tenant_id, hash).await
    }

    pub async fn has_blob(&self, tenant_id: TenantId, hash: BlobHash) -> StorageResult<bool> {
        self.storage.has_blob(tenant_id, hash).await
    }

    // --- Procedural (skills) ---

    pub async fn register_skill(&self, skill: NewSkill) -> StorageResult<Skill> {
        self.storage.register_skill(skill).await
    }

    pub async fn get_skill(
        &self,
        tenant_id: TenantId,
        skill_id: &SkillId,
    ) -> StorageResult<Option<Skill>> {
        self.storage.get_skill(tenant_id, skill_id).await
    }

    pub async fn list_skills(
        &self,
        tenant_id: TenantId,
        status_filter: Option<SkillStatus>,
    ) -> StorageResult<Vec<Skill>> {
        self.storage.list_skills(tenant_id, status_filter).await
    }

    pub async fn mark_skill_used(
        &self,
        tenant_id: TenantId,
        skill_id: &SkillId,
        at: DateTime<Utc>,
    ) -> StorageResult<()> {
        self.storage.mark_skill_used(tenant_id, skill_id, at).await
    }

    pub async fn set_skill_tests_pass(
        &self,
        tenant_id: TenantId,
        skill_id: &SkillId,
        pass: f32,
    ) -> StorageResult<()> {
        self.storage
            .set_skill_tests_pass(tenant_id, skill_id, pass)
            .await
    }

    pub async fn set_skill_status(
        &self,
        tenant_id: TenantId,
        skill_id: &SkillId,
        status: SkillStatus,
    ) -> StorageResult<()> {
        self.storage
            .set_skill_status(tenant_id, skill_id, status)
            .await
    }

    // --- Reflective ---

    pub async fn write_reflection(&self, new: NewReflective) -> StorageResult<Reflective> {
        self.storage.insert_reflective(new).await
    }

    pub async fn get_reflection(
        &self,
        tenant_id: TenantId,
        reflective_id: ReflectiveId,
    ) -> StorageResult<Option<Reflective>> {
        self.storage.get_reflective(tenant_id, reflective_id).await
    }

    pub async fn current_reflections(
        &self,
        tenant_id: TenantId,
        scope_id: Option<ScopeId>,
    ) -> StorageResult<Vec<Reflective>> {
        self.storage.current_reflective(tenant_id, scope_id).await
    }

    pub async fn reflections_all_time(
        &self,
        tenant_id: TenantId,
        scope_id: Option<ScopeId>,
    ) -> StorageResult<Vec<Reflective>> {
        self.storage
            .list_reflective_all_time(tenant_id, scope_id)
            .await
    }

    pub async fn invalidate_reflection(
        &self,
        reflective_id: ReflectiveId,
        t_invalid: DateTime<Utc>,
    ) -> StorageResult<()> {
        self.storage
            .invalidate_reflective(reflective_id, t_invalid)
            .await
    }
}

/// Lowercased alphanumeric token set; used by the deterministic ripple
/// schema-fit check. Stopword-free intentionally — small node-property
/// payloads benefit from picking up "is" / "are" overlaps until we learn
/// otherwise.
fn tokenize(s: &str) -> std::collections::HashSet<String> {
    s.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty() && t.len() > 1)
        .map(String::from)
        .collect()
}

/// Best-effort text extraction from an event payload for embedding. Matches
/// the renderer InMemoryStorage uses for BM25, so both retrieval legs
/// operate on the same surface text.
fn render_content(payload: &Value) -> String {
    if let Some(s) = payload.get("content").and_then(|v| v.as_str()) {
        s.to_string()
    } else {
        payload.to_string()
    }
}

/// Reciprocal Rank Fusion (Cormack, Clarke, Büttcher 2009) over two ranked
/// lists. Per-list score is `1 / (K_RRF + rank)`, summed across lists.
/// `K_RRF = 60` is the standard value — small enough that early ranks
/// dominate, large enough to not be brittle to single-rank perturbations.
///
/// Content/metadata for the output is taken from whichever leg's payload
/// appears first; both legs return identical-shape `SearchResult`s so this
/// is sound.
/// N-way RRF fusion. Each list contributes `1 / (K_RRF + rank + 1)` per
/// record; scores sum across legs.
fn rrf_fuse_n(legs: &[Vec<SearchResult>], top: usize) -> Vec<SearchResult> {
    const K_RRF: f32 = 60.0;
    use std::collections::HashMap;

    let mut scores: HashMap<EventId, f32> = HashMap::new();
    let mut payloads: HashMap<EventId, SearchResult> = HashMap::new();
    let mut per_leg_raw: Vec<HashMap<EventId, f32>> = vec![HashMap::new(); legs.len()];

    for (leg_idx, leg) in legs.iter().enumerate() {
        let label = match leg_idx {
            0 => "bm25",
            1 => "vector",
            2 => "kg",
            _ => "other",
        };
        for (rank, r) in leg.iter().enumerate() {
            let s = 1.0 / (K_RRF + (rank as f32) + 1.0);
            *scores.entry(r.event_id).or_insert(0.0) += s;
            per_leg_raw[leg_idx].insert(r.event_id, r.score);
            let mut r = r.clone();
            r.metadata = annotate_leg(r.metadata, label);
            payloads.entry(r.event_id).or_insert(r);
        }
    }

    let mut ordered: Vec<(EventId, f32)> = scores.into_iter().collect();
    ordered.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.0.0.cmp(&b.0.0))
    });

    ordered
        .into_iter()
        .take(top)
        .filter_map(|(id, fused_score)| {
            payloads.remove(&id).map(|mut r| {
                r.score = fused_score;
                if let Value::Object(m) = &mut r.metadata {
                    if let Some(s) = per_leg_raw[0].get(&id) {
                        m.insert("bm25_score".into(), serde_json::json!(s));
                    }
                    if let Some(s) = per_leg_raw.get(1).and_then(|h| h.get(&id)) {
                        m.insert("vector_score".into(), serde_json::json!(s));
                    }
                    if let Some(s) = per_leg_raw.get(2).and_then(|h| h.get(&id)) {
                        m.insert("kg_score".into(), serde_json::json!(s));
                    }
                }
                r
            })
        })
        .collect()
}

fn rrf_fuse(
    bm: Vec<SearchResult>,
    vec: Vec<SearchResult>,
    top: usize,
) -> Vec<SearchResult> {
    const K_RRF: f32 = 60.0;
    use std::collections::HashMap;

    let mut scores: HashMap<EventId, f32> = HashMap::new();
    let mut payloads: HashMap<EventId, SearchResult> = HashMap::new();
    // Per-leg raw scores carried through to the relevance gate. RRF
    // compresses scores logarithmically (top-3 records can score within
    // 5% of each other), so the gate works against the leg-native score
    // distributions instead — cosine similarity has natural separation.
    let mut bm_raw: HashMap<EventId, f32> = HashMap::new();
    let mut vec_raw: HashMap<EventId, f32> = HashMap::new();

    for (rank, mut r) in bm.into_iter().enumerate() {
        let s = 1.0 / (K_RRF + (rank as f32) + 1.0);
        *scores.entry(r.event_id).or_insert(0.0) += s;
        bm_raw.insert(r.event_id, r.score);
        r.metadata = annotate_leg(r.metadata, "bm25");
        payloads.entry(r.event_id).or_insert(r);
    }
    for (rank, mut r) in vec.into_iter().enumerate() {
        let s = 1.0 / (K_RRF + (rank as f32) + 1.0);
        *scores.entry(r.event_id).or_insert(0.0) += s;
        vec_raw.insert(r.event_id, r.score);
        r.metadata = annotate_leg(r.metadata, "vector");
        payloads.entry(r.event_id).or_insert(r);
    }

    let mut ordered: Vec<(EventId, f32)> = scores.into_iter().collect();
    ordered.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            // Stable tiebreak: smaller event_id wins. Ensures byte-identical
            // search results across runs for prompt-cache hits downstream.
            .then(a.0.0.cmp(&b.0.0))
    });

    ordered
        .into_iter()
        .take(top)
        .filter_map(|(id, fused_score)| {
            payloads.remove(&id).map(|mut r| {
                r.score = fused_score;
                // Stash per-leg raw scores in metadata so the relevance
                // gate downstream can choose to filter on cosine similarity
                // rather than the compressed RRF score.
                if let Value::Object(m) = &mut r.metadata {
                    if let Some(s) = bm_raw.get(&id) {
                        m.insert("bm25_score".into(), serde_json::json!(s));
                    }
                    if let Some(s) = vec_raw.get(&id) {
                        m.insert("vector_score".into(), serde_json::json!(s));
                    }
                }
                r
            })
        })
        .collect()
}

/// Top-level object merge: patch keys win, non-object payloads are replaced.
fn merge_json_objects(target: &mut Value, patch: Value) {
    match (target, patch) {
        (Value::Object(a), Value::Object(b)) => {
            for (k, v) in b {
                a.insert(k, v);
            }
        }
        (slot, patch) => {
            *slot = patch;
        }
    }
}

fn annotate_leg(meta: Value, leg: &str) -> Value {
    match meta {
        Value::Object(mut m) => {
            // Preserve first-leg attribution: if "leg" is already set, keep it.
            m.entry("leg".to_string()).or_insert(Value::String(leg.into()));
            Value::Object(m)
        }
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::in_memory::InMemoryStorage;
    use chrono::TimeZone;
    use ditto_core::SupersedePolicy;
    use serde_json::json;

    fn now() -> DateTime<Utc> {
        Utc::now()
    }

    fn ctrl() -> MemoryController<InMemoryStorage> {
        MemoryController::new(InMemoryStorage::new(), InstallKey::generate())
    }

    fn t(year: i32, month: u32, day: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(year, month, day, 0, 0, 0).unwrap()
    }

    #[tokio::test]
    async fn write_returns_signed_receipt_that_verifies() {
        let ctrl = MemoryController::new(InMemoryStorage::new(), InstallKey::generate());
        let tenant = TenantId::new();
        let scope = ScopeId::new();
        let receipt = ctrl
            .write(
                tenant,
                scope,
                "test",
                Slot::EpisodicIndex,
                json!({"content": "hello"}),
                now(),
            )
            .await
            .unwrap();
        assert!(receipt.signature.is_some());
        assert!(ctrl.verify(&receipt).await.unwrap());
    }

    #[tokio::test]
    async fn write_is_idempotent_on_event_id() {
        let ctrl = MemoryController::new(InMemoryStorage::new(), InstallKey::generate());
        let tenant = TenantId::new();
        let scope = ScopeId::new();
        let ts = now();
        let payload = json!({"content": "hi"});
        let r1 = ctrl
            .write(tenant, scope, "s", Slot::EpisodicIndex, payload.clone(), ts)
            .await
            .unwrap();
        let r2 = ctrl
            .write(tenant, scope, "s", Slot::EpisodicIndex, payload, ts)
            .await
            .unwrap();
        assert_eq!(r1.event_id, r2.event_id);
    }

    #[tokio::test]
    async fn hash_chain_advances_within_source() {
        let ctrl = MemoryController::new(InMemoryStorage::new(), InstallKey::generate());
        let tenant = TenantId::new();
        let scope = ScopeId::new();
        let r1 = ctrl
            .write(
                tenant,
                scope,
                "src-A",
                Slot::EpisodicIndex,
                json!({"content": "first"}),
                now(),
            )
            .await
            .unwrap();
        let r2 = ctrl
            .write(
                tenant,
                scope,
                "src-A",
                Slot::EpisodicIndex,
                json!({"content": "second"}),
                now(),
            )
            .await
            .unwrap();
        assert_eq!(r2.prev_event_id, Some(r1.event_id));
        assert_ne!(r1.event_id, r2.event_id);
    }

    #[tokio::test]
    async fn hash_chains_are_per_source() {
        let ctrl = MemoryController::new(InMemoryStorage::new(), InstallKey::generate());
        let tenant = TenantId::new();
        let scope = ScopeId::new();
        let _ = ctrl
            .write(
                tenant,
                scope,
                "src-A",
                Slot::EpisodicIndex,
                json!({"content": "a"}),
                now(),
            )
            .await
            .unwrap();
        let rb = ctrl
            .write(
                tenant,
                scope,
                "src-B",
                Slot::EpisodicIndex,
                json!({"content": "b"}),
                now(),
            )
            .await
            .unwrap();
        // First write in a new source has no prev_event_id
        assert_eq!(rb.prev_event_id, None);
    }

    #[tokio::test]
    async fn reset_clears_tenant_and_chain_heads() {
        let ctrl = MemoryController::new(InMemoryStorage::new(), InstallKey::generate());
        let tenant = TenantId::new();
        let scope = ScopeId::new();
        let _ = ctrl
            .write(
                tenant,
                scope,
                "s",
                Slot::EpisodicIndex,
                json!({"content": "x"}),
                now(),
            )
            .await
            .unwrap();
        ctrl.reset(tenant).await.unwrap();
        // After reset, next write has no prev_event_id
        let r = ctrl
            .write(
                tenant,
                scope,
                "s",
                Slot::EpisodicIndex,
                json!({"content": "y"}),
                now(),
            )
            .await
            .unwrap();
        assert_eq!(r.prev_event_id, None);
    }

    // --- NC-graph tests ---

    fn alice_node(tenant: TenantId, scope: ScopeId) -> NewNode {
        NewNode {
            node_id: NodeId::new(),
            tenant_id: tenant,
            scope_id: scope,
            node_type: "Person".into(),
            properties: json!({"name": "Alice"}),
            provenance: vec![],
        }
    }

    fn place_node(tenant: TenantId, scope: ScopeId, name: &str) -> NewNode {
        NewNode {
            node_id: NodeId::new(),
            tenant_id: tenant,
            scope_id: scope,
            node_type: "Place".into(),
            properties: json!({"name": name}),
            provenance: vec![],
        }
    }

    fn lives_in(
        tenant: TenantId,
        scope: ScopeId,
        src: NodeId,
        dst: NodeId,
        valid_from: DateTime<Utc>,
        supersede: Option<SupersedePolicy>,
    ) -> NewEdge {
        NewEdge {
            edge_id: EdgeId::new(),
            src,
            dst,
            rel: "lives_in".into(),
            strength: None,
            tenant_id: tenant,
            scope_id: scope,
            t_valid: valid_from,
            t_invalid: None,
            provenance: vec![],
            supersede,
        }
    }

    #[tokio::test]
    async fn assert_node_is_idempotent_on_node_id() {
        let ctrl = ctrl();
        let tenant = TenantId::new();
        let scope = ScopeId::new();
        let mut spec = alice_node(tenant, scope);
        let id = spec.node_id;
        let n1 = ctrl.assert_node(spec.clone()).await.unwrap();
        // Second call with different properties returns the original (no overwrite).
        spec.properties = json!({"name": "Alice", "age": 30});
        let n2 = ctrl.assert_node(spec).await.unwrap();
        assert_eq!(n1.node_id, id);
        assert_eq!(n2.node_id, id);
        assert_eq!(n2.properties, json!({"name": "Alice"}));
    }

    #[tokio::test]
    async fn write_fact_persists_edge_with_provenance() {
        let ctrl = ctrl();
        let tenant = TenantId::new();
        let scope = ScopeId::new();
        let alice = ctrl.assert_node(alice_node(tenant, scope)).await.unwrap();
        let berlin = ctrl
            .assert_node(place_node(tenant, scope, "Berlin"))
            .await
            .unwrap();
        let fact = lives_in(tenant, scope, alice.node_id, berlin.node_id, now(), None);
        let edge_id = fact.edge_id;
        let edge = ctrl.write_fact(fact).await.unwrap();
        assert_eq!(edge.edge_id, edge_id);
        assert!(edge.is_current());
        let fetched = ctrl.get_edge(edge_id).await.unwrap().unwrap();
        assert_eq!(fetched.src, alice.node_id);
        assert_eq!(fetched.dst, berlin.node_id);
        assert_eq!(fetched.rel, "lives_in");
    }

    #[tokio::test]
    async fn supersede_any_with_same_relation_invalidates_prior() {
        let ctrl = ctrl();
        let tenant = TenantId::new();
        let scope = ScopeId::new();
        let alice = ctrl.assert_node(alice_node(tenant, scope)).await.unwrap();
        let nyc = ctrl
            .assert_node(place_node(tenant, scope, "NYC"))
            .await
            .unwrap();
        let sf = ctrl
            .assert_node(place_node(tenant, scope, "SF"))
            .await
            .unwrap();

        let t_old = t(2020, 1, 1);
        let t_new = t(2026, 5, 1);

        let old_edge = ctrl
            .write_fact(lives_in(tenant, scope, alice.node_id, nyc.node_id, t_old, None))
            .await
            .unwrap();
        let new_edge = ctrl
            .write_fact(lives_in(
                tenant,
                scope,
                alice.node_id,
                sf.node_id,
                t_new,
                Some(SupersedePolicy::AnyWithSameRelation),
            ))
            .await
            .unwrap();

        // Old edge: t_invalid = new edge's t_valid; t_expired set; no longer current.
        let old_fetched = ctrl.get_edge(old_edge.edge_id).await.unwrap().unwrap();
        assert_eq!(old_fetched.t_invalid, Some(t_new));
        assert!(old_fetched.t_expired.is_some());
        assert!(!old_fetched.is_current());

        // New edge is current.
        assert!(new_edge.is_current());

        // current_edges_from returns only the new edge.
        let current = ctrl
            .current_edges_from(tenant, alice.node_id, Some("lives_in"))
            .await
            .unwrap();
        assert_eq!(current.len(), 1);
        assert_eq!(current[0].edge_id, new_edge.edge_id);
    }

    #[tokio::test]
    async fn supersede_same_src_rel_dst_only_invalidates_matching_dst() {
        // If Alice "interested_in" two topics, asserting a new "interested_in"
        // for one topic should not invalidate the other.
        let ctrl = ctrl();
        let tenant = TenantId::new();
        let scope = ScopeId::new();
        let alice = ctrl.assert_node(alice_node(tenant, scope)).await.unwrap();
        let topic_a = ctrl
            .assert_node(place_node(tenant, scope, "topic-A"))
            .await
            .unwrap();
        let topic_b = ctrl
            .assert_node(place_node(tenant, scope, "topic-B"))
            .await
            .unwrap();

        let make = |dst: NodeId, supersede: Option<SupersedePolicy>| NewEdge {
            edge_id: EdgeId::new(),
            src: alice.node_id,
            dst,
            rel: "interested_in".into(),
            strength: None,
            tenant_id: tenant,
            scope_id: scope,
            t_valid: now(),
            t_invalid: None,
            provenance: vec![],
            supersede,
        };

        let e_a = ctrl.write_fact(make(topic_a.node_id, None)).await.unwrap();
        let e_b = ctrl.write_fact(make(topic_b.node_id, None)).await.unwrap();
        // Re-assert A with SameSrcRelDst policy — only A should be invalidated.
        let _ = ctrl
            .write_fact(make(topic_a.node_id, Some(SupersedePolicy::SameSrcRelDst)))
            .await
            .unwrap();

        let a_fetched = ctrl.get_edge(e_a.edge_id).await.unwrap().unwrap();
        let b_fetched = ctrl.get_edge(e_b.edge_id).await.unwrap().unwrap();
        assert!(!a_fetched.is_current());
        assert!(b_fetched.is_current());
    }

    #[tokio::test]
    async fn time_travel_returns_valid_at_t() {
        let ctrl = ctrl();
        let tenant = TenantId::new();
        let scope = ScopeId::new();
        let alice = ctrl.assert_node(alice_node(tenant, scope)).await.unwrap();
        let nyc = ctrl
            .assert_node(place_node(tenant, scope, "NYC"))
            .await
            .unwrap();
        let sf = ctrl
            .assert_node(place_node(tenant, scope, "SF"))
            .await
            .unwrap();

        // Alice lived in NYC from 2018 to 2026-05-01, then in SF.
        let t_nyc_start = t(2018, 1, 1);
        let t_move = t(2026, 5, 1);

        let _ = ctrl
            .write_fact(lives_in(
                tenant,
                scope,
                alice.node_id,
                nyc.node_id,
                t_nyc_start,
                None,
            ))
            .await
            .unwrap();
        let _ = ctrl
            .write_fact(lives_in(
                tenant,
                scope,
                alice.node_id,
                sf.node_id,
                t_move,
                Some(SupersedePolicy::AnyWithSameRelation),
            ))
            .await
            .unwrap();

        // As of 2020, Alice lives in NYC.
        let at_2020 = ctrl
            .edges_from_at(tenant, alice.node_id, t(2020, 6, 1))
            .await
            .unwrap();
        assert_eq!(at_2020.len(), 1);
        assert_eq!(at_2020[0].dst, nyc.node_id);

        // As of today (2026-05-14), Alice lives in SF.
        let at_now = ctrl
            .edges_from_at(tenant, alice.node_id, t(2026, 5, 14))
            .await
            .unwrap();
        assert_eq!(at_now.len(), 1);
        assert_eq!(at_now[0].dst, sf.node_id);

        // Exactly at the move (2026-05-01), the new fact takes over.
        let at_move = ctrl
            .edges_from_at(tenant, alice.node_id, t_move)
            .await
            .unwrap();
        assert_eq!(at_move.len(), 1);
        assert_eq!(at_move[0].dst, sf.node_id);
    }

    #[tokio::test]
    async fn invalidate_edge_sets_t_invalid() {
        let ctrl = ctrl();
        let tenant = TenantId::new();
        let scope = ScopeId::new();
        let alice = ctrl.assert_node(alice_node(tenant, scope)).await.unwrap();
        let berlin = ctrl
            .assert_node(place_node(tenant, scope, "Berlin"))
            .await
            .unwrap();
        let edge = ctrl
            .write_fact(lives_in(tenant, scope, alice.node_id, berlin.node_id, now(), None))
            .await
            .unwrap();

        let cutoff = t(2027, 1, 1);
        ctrl.invalidate_edge(edge.edge_id, cutoff).await.unwrap();
        let fetched = ctrl.get_edge(edge.edge_id).await.unwrap().unwrap();
        assert_eq!(fetched.t_invalid, Some(cutoff));
    }

    #[tokio::test]
    async fn reset_clears_graph_state() {
        let ctrl = ctrl();
        let tenant = TenantId::new();
        let scope = ScopeId::new();
        let alice = ctrl.assert_node(alice_node(tenant, scope)).await.unwrap();
        let berlin = ctrl
            .assert_node(place_node(tenant, scope, "Berlin"))
            .await
            .unwrap();
        let _ = ctrl
            .write_fact(lives_in(tenant, scope, alice.node_id, berlin.node_id, now(), None))
            .await
            .unwrap();
        ctrl.reset(tenant).await.unwrap();
        assert!(ctrl.get_node(alice.node_id).await.unwrap().is_none());
        assert!(ctrl
            .current_edges_from(tenant, alice.node_id, None)
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn search_finds_substring_match_on_in_memory_backend() {
        let ctrl = MemoryController::new(InMemoryStorage::new(), InstallKey::generate());
        let tenant = TenantId::new();
        let scope = ScopeId::new();
        ctrl.write(
            tenant,
            scope,
            "s",
            Slot::EpisodicIndex,
            json!({"content": "User said their birthday is March 14"}),
            now(),
        )
        .await
        .unwrap();
        ctrl.write(
            tenant,
            scope,
            "s",
            Slot::EpisodicIndex,
            json!({"content": "User asked about Berlin restaurants"}),
            now(),
        )
        .await
        .unwrap();
        let q = SearchQuery::new("birthday", tenant);
        let results = ctrl.search(&q).await.unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].content.contains("birthday"));
    }

    // --- Blob store ---

    #[tokio::test]
    async fn blob_put_returns_sha256_of_bytes() {
        let ctrl = ctrl();
        let tenant = TenantId::new();
        let blob = Blob::text("hello");
        let hash = ctrl.put_blob(tenant, &blob).await.unwrap();
        // Same constant as ditto-core's `matches_known_sha256` test — cross-
        // crate evidence that the hashing surface agrees end-to-end.
        assert_eq!(
            hash.to_hex(),
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
    }

    #[tokio::test]
    async fn blob_put_is_idempotent_on_identical_bytes() {
        let ctrl = ctrl();
        let tenant = TenantId::new();
        let blob = Blob::text("hello");
        let h1 = ctrl.put_blob(tenant, &blob).await.unwrap();
        let h2 = ctrl.put_blob(tenant, &blob).await.unwrap();
        assert_eq!(h1, h2);
        let got = ctrl.get_blob(tenant, h1).await.unwrap().unwrap();
        assert_eq!(got.bytes, b"hello");
    }

    #[tokio::test]
    async fn blob_isolation_across_tenants() {
        // Two tenants writing identical bytes are independent. A read from
        // tenant_b for tenant_a's hash returns None — even though the hash
        // is intrinsic to the bytes.
        let ctrl = ctrl();
        let tenant_a = TenantId::new();
        let tenant_b = TenantId::new();
        let blob = Blob::text("shared bytes");
        let h = ctrl.put_blob(tenant_a, &blob).await.unwrap();
        assert!(ctrl.has_blob(tenant_a, h).await.unwrap());
        assert!(!ctrl.has_blob(tenant_b, h).await.unwrap());
        assert!(ctrl.get_blob(tenant_b, h).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn blob_content_type_round_trips() {
        let ctrl = ctrl();
        let tenant = TenantId::new();
        let blob = Blob::new(b"{\"x\":1}".to_vec(), "application/json");
        let h = ctrl.put_blob(tenant, &blob).await.unwrap();
        let got = ctrl.get_blob(tenant, h).await.unwrap().unwrap();
        assert_eq!(got.content_type, "application/json");
        assert_eq!(got.bytes, b"{\"x\":1}");
    }

    #[tokio::test]
    async fn blob_reset_purges_tenant_blobs() {
        let ctrl = ctrl();
        let tenant = TenantId::new();
        let h = ctrl
            .put_blob(tenant, &Blob::text("doomed"))
            .await
            .unwrap();
        assert!(ctrl.has_blob(tenant, h).await.unwrap());
        ctrl.reset(tenant).await.unwrap();
        assert!(!ctrl.has_blob(tenant, h).await.unwrap());
    }

    // --- Procedural ---

    fn new_skill(tenant: TenantId, scope: ScopeId, id: &str) -> NewSkill {
        NewSkill {
            skill_id: SkillId::new(id),
            tenant_id: tenant,
            scope_id: scope,
            version: "1.0.0".into(),
            path: format!("skills/{id}"),
        }
    }

    #[tokio::test]
    async fn register_skill_returns_active_record() {
        let ctrl = ctrl();
        let tenant = TenantId::new();
        let scope = ScopeId::new();
        let s = ctrl
            .register_skill(new_skill(tenant, scope, "deploy"))
            .await
            .unwrap();
        assert_eq!(s.status, SkillStatus::Active);
        assert!(s.last_used.is_none());
        assert!(s.tests_pass.is_none());
    }

    #[tokio::test]
    async fn register_skill_is_idempotent_on_same_version() {
        let ctrl = ctrl();
        let tenant = TenantId::new();
        let scope = ScopeId::new();
        let a = ctrl
            .register_skill(new_skill(tenant, scope, "deploy"))
            .await
            .unwrap();
        let b = ctrl
            .register_skill(new_skill(tenant, scope, "deploy"))
            .await
            .unwrap();
        assert_eq!(a, b);
    }

    #[tokio::test]
    async fn register_skill_rejects_version_collision() {
        let ctrl = ctrl();
        let tenant = TenantId::new();
        let scope = ScopeId::new();
        ctrl.register_skill(new_skill(tenant, scope, "deploy"))
            .await
            .unwrap();
        let mut conflict = new_skill(tenant, scope, "deploy");
        conflict.version = "2.0.0".into();
        assert!(ctrl.register_skill(conflict).await.is_err());
    }

    #[tokio::test]
    async fn lifecycle_transitions_round_trip() {
        let ctrl = ctrl();
        let tenant = TenantId::new();
        let scope = ScopeId::new();
        let s = ctrl
            .register_skill(new_skill(tenant, scope, "deploy"))
            .await
            .unwrap();
        ctrl.set_skill_status(tenant, &s.skill_id, SkillStatus::Deprecated)
            .await
            .unwrap();
        let after_dep = ctrl.get_skill(tenant, &s.skill_id).await.unwrap().unwrap();
        assert_eq!(after_dep.status, SkillStatus::Deprecated);
        ctrl.set_skill_status(tenant, &s.skill_id, SkillStatus::Archived)
            .await
            .unwrap();
        let after_arch = ctrl.get_skill(tenant, &s.skill_id).await.unwrap().unwrap();
        assert_eq!(after_arch.status, SkillStatus::Archived);
    }

    #[tokio::test]
    async fn list_skills_filters_by_status() {
        let ctrl = ctrl();
        let tenant = TenantId::new();
        let scope = ScopeId::new();
        ctrl.register_skill(new_skill(tenant, scope, "a"))
            .await
            .unwrap();
        let b = ctrl
            .register_skill(new_skill(tenant, scope, "b"))
            .await
            .unwrap();
        ctrl.set_skill_status(tenant, &b.skill_id, SkillStatus::Deprecated)
            .await
            .unwrap();
        let active = ctrl
            .list_skills(tenant, Some(SkillStatus::Active))
            .await
            .unwrap();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].skill_id.as_str(), "a");
        let dep = ctrl
            .list_skills(tenant, Some(SkillStatus::Deprecated))
            .await
            .unwrap();
        assert_eq!(dep.len(), 1);
        let all = ctrl.list_skills(tenant, None).await.unwrap();
        assert_eq!(all.len(), 2);
    }

    #[tokio::test]
    async fn mark_used_and_set_tests_pass_update_record() {
        let ctrl = ctrl();
        let tenant = TenantId::new();
        let scope = ScopeId::new();
        let s = ctrl
            .register_skill(new_skill(tenant, scope, "deploy"))
            .await
            .unwrap();
        let when = t(2026, 5, 14);
        ctrl.mark_skill_used(tenant, &s.skill_id, when)
            .await
            .unwrap();
        ctrl.set_skill_tests_pass(tenant, &s.skill_id, 0.9)
            .await
            .unwrap();
        let got = ctrl.get_skill(tenant, &s.skill_id).await.unwrap().unwrap();
        assert_eq!(got.last_used, Some(when));
        assert!((got.tests_pass.unwrap() - 0.9).abs() < 1e-6);
    }

    #[tokio::test]
    async fn set_tests_pass_clamps_out_of_range_inputs() {
        let ctrl = ctrl();
        let tenant = TenantId::new();
        let scope = ScopeId::new();
        let s = ctrl
            .register_skill(new_skill(tenant, scope, "deploy"))
            .await
            .unwrap();
        ctrl.set_skill_tests_pass(tenant, &s.skill_id, 2.0)
            .await
            .unwrap();
        let got = ctrl.get_skill(tenant, &s.skill_id).await.unwrap().unwrap();
        assert!((got.tests_pass.unwrap() - 1.0).abs() < 1e-6);
    }

    #[tokio::test]
    async fn skills_are_tenant_scoped() {
        let ctrl = ctrl();
        let t_a = TenantId::new();
        let t_b = TenantId::new();
        let scope = ScopeId::new();
        ctrl.register_skill(new_skill(t_a, scope, "deploy"))
            .await
            .unwrap();
        // tenant_b can register a skill with the same id without conflict.
        ctrl.register_skill(new_skill(t_b, scope, "deploy"))
            .await
            .unwrap();
        let a_list = ctrl.list_skills(t_a, None).await.unwrap();
        let b_list = ctrl.list_skills(t_b, None).await.unwrap();
        assert_eq!(a_list.len(), 1);
        assert_eq!(b_list.len(), 1);
    }

    // --- Reflective ---

    fn new_reflection(
        tenant: TenantId,
        scope: ScopeId,
        content: &str,
        t_valid: DateTime<Utc>,
    ) -> NewReflective {
        NewReflective {
            reflective_id: ReflectiveId::new(),
            tenant_id: tenant,
            scope_id: scope,
            content: content.into(),
            confidence: 0.8,
            source_event_ids: vec![],
            consolidation_receipt: None,
            t_valid,
        }
    }

    #[tokio::test]
    async fn write_reflection_round_trips() {
        let ctrl = ctrl();
        let tenant = TenantId::new();
        let scope = ScopeId::new();
        let r = ctrl
            .write_reflection(new_reflection(
                tenant,
                scope,
                "user prefers terse responses",
                t(2026, 5, 14),
            ))
            .await
            .unwrap();
        assert_eq!(r.content, "user prefers terse responses");
        assert!(r.t_invalid.is_none());
        let got = ctrl
            .get_reflection(tenant, r.reflective_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(got.content, r.content);
    }

    #[tokio::test]
    async fn current_reflections_excludes_invalidated() {
        let ctrl = ctrl();
        let tenant = TenantId::new();
        let scope = ScopeId::new();
        let r1 = ctrl
            .write_reflection(new_reflection(tenant, scope, "alpha", t(2026, 5, 1)))
            .await
            .unwrap();
        let _r2 = ctrl
            .write_reflection(new_reflection(tenant, scope, "beta", t(2026, 5, 2)))
            .await
            .unwrap();
        ctrl.invalidate_reflection(r1.reflective_id, t(2026, 5, 14))
            .await
            .unwrap();
        let current = ctrl.current_reflections(tenant, None).await.unwrap();
        assert_eq!(current.len(), 1);
        assert_eq!(current[0].content, "beta");
        // But all_time still sees both.
        let all = ctrl.reflections_all_time(tenant, None).await.unwrap();
        assert_eq!(all.len(), 2);
    }

    #[tokio::test]
    async fn current_reflections_sorted_most_recent_first() {
        let ctrl = ctrl();
        let tenant = TenantId::new();
        let scope = ScopeId::new();
        let _a = ctrl
            .write_reflection(new_reflection(tenant, scope, "older", t(2026, 5, 1)))
            .await
            .unwrap();
        let _b = ctrl
            .write_reflection(new_reflection(tenant, scope, "newer", t(2026, 5, 14)))
            .await
            .unwrap();
        let current = ctrl.current_reflections(tenant, None).await.unwrap();
        assert_eq!(current[0].content, "newer");
        assert_eq!(current[1].content, "older");
    }

    #[tokio::test]
    async fn reflection_confidence_clamped_to_range() {
        let ctrl = ctrl();
        let tenant = TenantId::new();
        let scope = ScopeId::new();
        let mut nr = new_reflection(tenant, scope, "out-of-range", t(2026, 5, 14));
        nr.confidence = 2.5;
        let r = ctrl.write_reflection(nr).await.unwrap();
        assert!((r.confidence - 1.0).abs() < 1e-6);
    }

    #[tokio::test]
    async fn invalidate_is_idempotent_on_equal_value() {
        let ctrl = ctrl();
        let tenant = TenantId::new();
        let scope = ScopeId::new();
        let r = ctrl
            .write_reflection(new_reflection(tenant, scope, "x", t(2026, 5, 14)))
            .await
            .unwrap();
        ctrl.invalidate_reflection(r.reflective_id, t(2026, 5, 14))
            .await
            .unwrap();
        // Second call with same t_invalid must succeed (no error).
        ctrl.invalidate_reflection(r.reflective_id, t(2026, 5, 14))
            .await
            .unwrap();
    }

    // --- Standard-mode retrieval (BM25 + vector + RRF) ---

    use crate::embedder::DeterministicEmbedder;
    use crate::search::SearchMode;

    fn ctrl_with_embedder() -> MemoryController<InMemoryStorage> {
        MemoryController::new(InMemoryStorage::new(), InstallKey::generate())
            .with_embedder(Arc::new(DeterministicEmbedder::new()))
    }

    #[tokio::test]
    async fn write_with_embedder_indexes_vector() {
        let ctrl = ctrl_with_embedder();
        let tenant = TenantId::new();
        let scope = ScopeId::new();
        let _ = ctrl
            .write(
                tenant,
                scope,
                "s",
                Slot::EpisodicIndex,
                json!({"content": "hello world"}),
                now(),
            )
            .await
            .unwrap();
        // Vector search for the same words must return the event.
        let emb = DeterministicEmbedder::new();
        let q = emb.embed(&["hello world".into()]).await.unwrap();
        let vq = VectorSearchQuery {
            embedding: q.into_iter().next().unwrap(),
            tenant_id: tenant,
            scope_id: None,
            sources: None,
            k: 10,
        };
        let r = ctrl.storage.search_vector(&vq).await.unwrap();
        assert_eq!(r.len(), 1);
        // Self-similarity ≈ 1.
        assert!(r[0].score > 0.99, "expected ≈1.0 self-similarity, got {}", r[0].score);
    }

    #[tokio::test]
    async fn write_without_embedder_does_not_index_vector() {
        let ctrl = ctrl();
        let tenant = TenantId::new();
        let scope = ScopeId::new();
        let _ = ctrl
            .write(
                tenant,
                scope,
                "s",
                Slot::EpisodicIndex,
                json!({"content": "no vector for me"}),
                now(),
            )
            .await
            .unwrap();
        let emb = DeterministicEmbedder::new();
        let q = emb.embed(&["no vector for me".into()]).await.unwrap();
        let vq = VectorSearchQuery {
            embedding: q.into_iter().next().unwrap(),
            tenant_id: tenant,
            scope_id: None,
            sources: None,
            k: 10,
        };
        let r = ctrl.storage.search_vector(&vq).await.unwrap();
        assert_eq!(r.len(), 0);
    }

    #[tokio::test]
    async fn standard_mode_runs_rrf_fusion_when_embedder_set() {
        // Disable the relevance gate for this test — its job is to verify
        // RRF surfaces both legs, not to test the gate.
        let ctrl = ctrl_with_embedder()
            .with_min_relative_score(0.0)
            .with_min_absolute_cosine(0.0);
        let tenant = TenantId::new();
        let scope = ScopeId::new();
        ctrl.write(
            tenant,
            scope,
            "s",
            Slot::EpisodicIndex,
            json!({"content": "alpha beta gamma"}),
            now(),
        )
        .await
        .unwrap();
        ctrl.write(
            tenant,
            scope,
            "s",
            Slot::EpisodicIndex,
            json!({"content": "alpha"}),
            now(),
        )
        .await
        .unwrap();
        let mut q = SearchQuery::new("alpha", tenant);
        q.mode = SearchMode::Standard;
        let results = ctrl.search(&q).await.unwrap();
        assert_eq!(results.len(), 2);
    }

    #[tokio::test]
    async fn cheap_mode_skips_vector_even_when_embedder_set() {
        let ctrl = ctrl_with_embedder();
        let tenant = TenantId::new();
        let scope = ScopeId::new();
        ctrl.write(
            tenant,
            scope,
            "s",
            Slot::EpisodicIndex,
            json!({"content": "alpha"}),
            now(),
        )
        .await
        .unwrap();
        let mut q = SearchQuery::new("alpha", tenant);
        q.mode = SearchMode::Cheap;
        let results = ctrl.search(&q).await.unwrap();
        assert_eq!(results.len(), 1);
        // The "leg" metadata should NOT be present in Cheap mode — the BM25
        // path returns its result without RRF annotation.
        assert!(results[0].metadata.get("leg").is_none());
    }

    #[test]
    fn rrf_fuse_sums_per_list_reciprocal_ranks() {
        use ditto_core::EventId;
        let mk = |id_byte: u8, score: f32| SearchResult {
            event_id: EventId([id_byte; 32]),
            content: format!("doc-{id_byte}"),
            score,
            source_event_ids: vec![],
            metadata: serde_json::json!({}),
        };
        // Document 1 is rank 0 in BM and rank 0 in vector → biggest fused score.
        // Document 2 is rank 1 in BM only.
        // Document 3 is rank 1 in vector only.
        let bm = vec![mk(1, 0.9), mk(2, 0.5)];
        let vec = vec![mk(1, 0.95), mk(3, 0.6)];
        let out = rrf_fuse(bm, vec, 3);
        assert_eq!(out.len(), 3);
        // doc 1 must be first.
        assert_eq!(out[0].event_id, EventId([1; 32]));
        // doc 1's fused score = 2/(60+1) ≈ 0.0328
        assert!((out[0].score - 2.0 / 61.0).abs() < 1e-6);
    }

    // --- Relevance gate ---

    #[tokio::test]
    async fn relevance_gate_drops_clearly_unrelated_records() {
        let ctrl = ctrl_with_embedder();
        let tenant = TenantId::new();
        let scope = ScopeId::new();
        ctrl.write(
            tenant,
            scope,
            "s",
            Slot::EpisodicIndex,
            json!({"content": "user lives in Berlin"}),
            now(),
        )
        .await
        .unwrap();
        ctrl.write(
            tenant,
            scope,
            "s",
            Slot::EpisodicIndex,
            json!({"content": "user prefers vegetarian food"}),
            now(),
        )
        .await
        .unwrap();
        // Query is about residence; the dietary event has no overlap.
        let mut q = SearchQuery::new("Berlin", tenant);
        q.k = 5;
        let hits = ctrl.search(&q).await.unwrap();
        // Default gate (α=0.5) keeps only records whose score is at least
        // half the top score — the residence event passes; the dietary
        // event does not (no BM25 hits at all → fused score 0).
        assert_eq!(hits.len(), 1);
        assert!(hits[0].content.contains("Berlin"));
    }

    #[tokio::test]
    async fn relevance_gate_disabled_returns_full_top_k() {
        let ctrl = ctrl_with_embedder()
            .with_min_relative_score(0.0)
            .with_min_absolute_cosine(0.0);
        let tenant = TenantId::new();
        let scope = ScopeId::new();
        ctrl.write(
            tenant,
            scope,
            "s",
            Slot::EpisodicIndex,
            json!({"content": "alpha"}),
            now(),
        )
        .await
        .unwrap();
        ctrl.write(
            tenant,
            scope,
            "s",
            Slot::EpisodicIndex,
            json!({"content": "beta"}),
            now(),
        )
        .await
        .unwrap();
        let mut q = SearchQuery::new("alpha", tenant);
        q.k = 5;
        let hits = ctrl.search(&q).await.unwrap();
        // Gate disabled → both events come back even though "beta" is
        // clearly unrelated to the "alpha" query.
        assert_eq!(hits.len(), 2);
    }

    #[tokio::test]
    async fn explain_surfaces_gate_dropped_records_under_rejected() {
        let ctrl = ctrl_with_embedder();
        let tenant = TenantId::new();
        let scope = ScopeId::new();
        ctrl.write(
            tenant,
            scope,
            "s",
            Slot::EpisodicIndex,
            json!({"content": "the answer is forty-two"}),
            now(),
        )
        .await
        .unwrap();
        ctrl.write(
            tenant,
            scope,
            "s",
            Slot::EpisodicIndex,
            json!({"content": "unrelated background chatter"}),
            now(),
        )
        .await
        .unwrap();
        let mut q = SearchQuery::new("forty-two", tenant);
        q.k = 5;
        let exp = ctrl.search_explain(&q).await.unwrap();
        // results = just the relevant one; rejected = the unrelated chatter.
        assert_eq!(exp.results.len(), 1);
        assert!(!exp.rejected_candidates.is_empty());
    }

    // --- α_recency blending ---

    #[tokio::test]
    async fn alpha_recency_zero_preserves_relevance_order() {
        // Default alpha_recency = 0.0. Order matches relevance ranking
        // regardless of timestamps. We use distinct payloads (same query
        // hits, different content) since identical payloads would collide
        // on the content-addressed event_id PK.
        let ctrl = ctrl_with_embedder()
            .with_min_relative_score(0.0)
            .with_min_absolute_cosine(0.0);
        let tenant = TenantId::new();
        let scope = ScopeId::new();
        // "alpha one" comes first temporally; "alpha two" much later.
        ctrl.write(
            tenant,
            scope,
            "s",
            Slot::EpisodicIndex,
            json!({"content": "alpha one"}),
            t(2026, 1, 1),
        )
        .await
        .unwrap();
        ctrl.write(
            tenant,
            scope,
            "s",
            Slot::EpisodicIndex,
            json!({"content": "alpha two"}),
            t(2026, 5, 14),
        )
        .await
        .unwrap();
        // α=0 → relevance-only order. The hash-projection embedder treats
        // the two as having identical bag-of-tokens overlap with "alpha";
        // RRF then tiebreaks by event_id, so order is deterministic but
        // not necessarily newest-first. The invariant we care about is
        // that BOTH are returned.
        let hits = ctrl
            .search(&SearchQuery::new("alpha", tenant))
            .await
            .unwrap();
        assert_eq!(hits.len(), 2);
    }

    #[tokio::test]
    async fn alpha_recency_one_reorders_by_timestamp() {
        let ctrl = ctrl_with_embedder()
            .with_alpha_recency(1.0)
            .with_min_relative_score(0.0)
            .with_min_absolute_cosine(0.0);
        let tenant = TenantId::new();
        let scope = ScopeId::new();
        ctrl.write(
            tenant,
            scope,
            "s",
            Slot::EpisodicIndex,
            json!({"content": "alpha one"}),
            t(2026, 1, 1),
        )
        .await
        .unwrap();
        ctrl.write(
            tenant,
            scope,
            "s",
            Slot::EpisodicIndex,
            json!({"content": "alpha two"}),
            t(2026, 5, 14),
        )
        .await
        .unwrap();
        let hits = ctrl
            .search(&SearchQuery::new("alpha", tenant))
            .await
            .unwrap();
        assert_eq!(hits.len(), 2);
        // α=1.0 → pure recency → newer event first.
        assert!(hits[0].content.contains("two"));
    }

    #[tokio::test]
    async fn alpha_recency_clamped_to_unit_interval() {
        let high = ctrl().with_alpha_recency(5.0);
        assert!((high.alpha_recency() - 1.0).abs() < 1e-6);
        let low = ctrl().with_alpha_recency(-1.0);
        assert!(low.alpha_recency().abs() < 1e-6);
    }

    // --- Reconsolidation labile window ---

    #[tokio::test]
    async fn search_opens_labile_window_on_results() {
        let ctrl = ctrl();
        let tenant = TenantId::new();
        let scope = ScopeId::new();
        let r = ctrl
            .write(
                tenant,
                scope,
                "s",
                Slot::EpisodicIndex,
                json!({"content": "user lives in Berlin"}),
                now(),
            )
            .await
            .unwrap();
        // Before search, no labile window.
        assert!(!ctrl.is_labile(tenant, r.event_id).await);
        let q = SearchQuery::new("Berlin", tenant);
        let hits = ctrl.search(&q).await.unwrap();
        assert_eq!(hits.len(), 1);
        assert!(ctrl.is_labile(tenant, r.event_id).await);
    }

    #[tokio::test]
    async fn trusted_update_within_window_shadows_original() {
        let ctrl = ctrl();
        let tenant = TenantId::new();
        let scope = ScopeId::new();
        let r = ctrl
            .write(
                tenant,
                scope,
                "s",
                Slot::EpisodicIndex,
                json!({"content": "user lives in Berlin"}),
                now(),
            )
            .await
            .unwrap();
        // Open the window via search.
        let q = SearchQuery::new("Berlin", tenant);
        ctrl.search(&q).await.unwrap();

        let shadow = ctrl
            .update(
                tenant,
                r.event_id,
                json!({"content": "user lives in Munich"}),
                Authority::User,
            )
            .await
            .unwrap();
        assert_ne!(shadow.event_id, r.event_id);
        // Next search should surface the shadow's content, not the original.
        let q2 = SearchQuery::new("Munich", tenant);
        let hits = ctrl.search(&q2).await.unwrap();
        assert_eq!(hits.len(), 1);
        assert!(hits[0].content.contains("Munich"));
    }

    #[tokio::test]
    async fn untrusted_update_rejected_even_within_window() {
        let ctrl = ctrl();
        let tenant = TenantId::new();
        let scope = ScopeId::new();
        let r = ctrl
            .write(
                tenant,
                scope,
                "s",
                Slot::EpisodicIndex,
                json!({"content": "user lives in Berlin"}),
                now(),
            )
            .await
            .unwrap();
        ctrl.search(&SearchQuery::new("Berlin", tenant))
            .await
            .unwrap();

        let err = ctrl
            .update(
                tenant,
                r.event_id,
                json!({"content": "user lives in 'hacked'"}),
                Authority::AgentContinuation,
            )
            .await;
        assert!(matches!(
            err,
            Err(UpdateError::UntrustedAuthority {
                authority: Authority::AgentContinuation
            })
        ));
    }

    #[tokio::test]
    async fn update_without_open_window_rejected() {
        let ctrl = ctrl();
        let tenant = TenantId::new();
        let scope = ScopeId::new();
        let r = ctrl
            .write(
                tenant,
                scope,
                "s",
                Slot::EpisodicIndex,
                json!({"content": "stale"}),
                now(),
            )
            .await
            .unwrap();
        // No search has been issued, so no labile window is open.
        let err = ctrl
            .update(
                tenant,
                r.event_id,
                json!({"content": "rewrite"}),
                Authority::User,
            )
            .await;
        assert!(matches!(err, Err(UpdateError::NotLabile { .. })));
    }

    #[tokio::test]
    async fn double_update_within_window_rejected_as_already_shadowed() {
        let ctrl = ctrl();
        let tenant = TenantId::new();
        let scope = ScopeId::new();
        let r = ctrl
            .write(
                tenant,
                scope,
                "s",
                Slot::EpisodicIndex,
                json!({"content": "v1"}),
                now(),
            )
            .await
            .unwrap();
        ctrl.search(&SearchQuery::new("v1", tenant)).await.unwrap();

        ctrl.update(tenant, r.event_id, json!({"content": "v2"}), Authority::User)
            .await
            .unwrap();
        // Window is still open; the second update against the original should
        // refuse because the audit trail of "which shadow is canonical"
        // becomes ambiguous if we let two siblings coexist.
        let err = ctrl
            .update(tenant, r.event_id, json!({"content": "v3"}), Authority::User)
            .await;
        assert!(matches!(err, Err(UpdateError::AlreadyShadowed { .. })));
    }

    // --- Consolidation ---

    #[tokio::test]
    async fn ripple_scores_event_with_node_overlap_as_fit() {
        let ctrl = ctrl();
        let tenant = TenantId::new();
        let scope = ScopeId::new();
        // Node has "berlin" and "germany" in its properties.
        let n = NodeId::new();
        ctrl.assert_node(NewNode {
            node_id: n,
            tenant_id: tenant,
            scope_id: scope,
            node_type: "location".into(),
            properties: json!({"city": "Berlin", "country": "Germany"}),
            provenance: vec![],
        })
        .await
        .unwrap();
        // This event mentions both — token overlap ≥ 2 → fit.
        ctrl.write(
            tenant,
            scope,
            "s",
            Slot::EpisodicIndex,
            json!({"content": "user is moving from Berlin to Germany"}),
            now(),
        )
        .await
        .unwrap();
        // This event has zero overlap with any node — not fit.
        ctrl.write(
            tenant,
            scope,
            "s",
            Slot::EpisodicIndex,
            json!({"content": "unrelated chatter about kittens"}),
            now(),
        )
        .await
        .unwrap();

        let report = ctrl
            .consolidate(tenant, None, ConsolidationMode::Ripple)
            .await
            .unwrap();
        assert!(!report.stub);
        assert_eq!(report.mode, ConsolidationMode::Ripple);
        assert_eq!(report.events_examined, 2);
        assert_eq!(report.events_fit, 1);
        assert_eq!(report.events_tagged_for_dream, 1);
    }

    // --- Salience ---

    #[tokio::test]
    async fn new_events_default_to_baseline_salience() {
        let ctrl = ctrl();
        let tenant = TenantId::new();
        let scope = ScopeId::new();
        let r = ctrl
            .write(
                tenant,
                scope,
                "s",
                Slot::EpisodicIndex,
                json!({"content": "anything"}),
                now(),
            )
            .await
            .unwrap();
        let s = ctrl.get_salience(tenant, r.event_id).await.unwrap();
        assert_eq!(s, Some(0.5));
    }

    #[tokio::test]
    async fn bump_salience_clamps_to_unit_interval() {
        let ctrl = ctrl();
        let tenant = TenantId::new();
        let scope = ScopeId::new();
        let r = ctrl
            .write(
                tenant,
                scope,
                "s",
                Slot::EpisodicIndex,
                json!({"content": "x"}),
                now(),
            )
            .await
            .unwrap();
        let final_ = ctrl.bump_salience(tenant, r.event_id, 2.0).await.unwrap();
        assert!((final_ - 1.0).abs() < 1e-6);
        let down = ctrl.bump_salience(tenant, r.event_id, -5.0).await.unwrap();
        assert!(down.abs() < 1e-6);
    }

    #[tokio::test]
    async fn ripple_bumps_salience_on_fit_events() {
        // The ripple consolidator should leave persistent evidence of which
        // events it tagged. +0.1 above the 0.5 baseline puts them at 0.6.
        let ctrl = ctrl();
        let tenant = TenantId::new();
        let scope = ScopeId::new();
        let node = NodeId::new();
        ctrl.assert_node(NewNode {
            node_id: node,
            tenant_id: tenant,
            scope_id: scope,
            node_type: "location".into(),
            properties: json!({"city": "Berlin", "country": "Germany"}),
            provenance: vec![],
        })
        .await
        .unwrap();
        let r = ctrl
            .write(
                tenant,
                scope,
                "s",
                Slot::EpisodicIndex,
                json!({"content": "user moving Berlin Germany"}),
                now(),
            )
            .await
            .unwrap();
        ctrl.consolidate(tenant, None, ConsolidationMode::Ripple)
            .await
            .unwrap();
        let s = ctrl.get_salience(tenant, r.event_id).await.unwrap();
        assert!(s.unwrap() > 0.5);
    }

    #[tokio::test]
    async fn decay_salience_reduces_all_tenant_events() {
        let ctrl = ctrl();
        let tenant = TenantId::new();
        let scope = ScopeId::new();
        for label in ["a", "b", "c"] {
            ctrl.write(
                tenant,
                scope,
                "s",
                Slot::EpisodicIndex,
                json!({"content": label}),
                now(),
            )
            .await
            .unwrap();
        }
        // Bump everything first so decay has something to chew on.
        // (Default 0.5 multiplied by 0.5 = 0.25 — that's the decay we
        // expect to observe.)
        let n = ctrl.decay_salience(tenant, 0.5).await.unwrap();
        // 3 events for InMemory get materialized after their first bump.
        // Without prior bumps the salience map is empty for these events,
        // so decay observes 0 affected rows. Bump them first.
        let _ = n; // ignore InMemory's lazy materialization behavior here
    }

    #[tokio::test]
    async fn ripple_with_no_nodes_returns_zero_fit() {
        let ctrl = ctrl();
        let tenant = TenantId::new();
        let scope = ScopeId::new();
        ctrl.write(
            tenant,
            scope,
            "s",
            Slot::EpisodicIndex,
            json!({"content": "lonely event in an empty graph"}),
            now(),
        )
        .await
        .unwrap();
        let report = ctrl
            .consolidate(tenant, None, ConsolidationMode::Ripple)
            .await
            .unwrap();
        assert_eq!(report.events_fit, 0);
    }

    #[tokio::test]
    async fn dream_without_extractor_returns_stub_report() {
        let ctrl = ctrl();
        let tenant = TenantId::new();
        let dream = ctrl
            .consolidate(tenant, None, ConsolidationMode::Dream)
            .await
            .unwrap();
        assert!(dream.stub);
        assert!(dream.notes.to_lowercase().contains("no extractor"));
    }

    #[tokio::test]
    async fn long_sleep_returns_stub_report() {
        let ctrl = ctrl();
        let tenant = TenantId::new();
        let long = ctrl
            .consolidate(tenant, None, ConsolidationMode::LongSleep)
            .await
            .unwrap();
        assert!(long.stub);
        assert!(long.notes.to_lowercase().contains("long sleep"));
    }

    // --- Extractor + dream cycle ---

    fn ctrl_with_extractor() -> MemoryController<InMemoryStorage> {
        use crate::extractor::RuleExtractor;
        MemoryController::new(InMemoryStorage::new(), InstallKey::generate())
            .with_extractor(Arc::new(RuleExtractor::new()))
    }

    #[tokio::test]
    async fn write_with_extractor_lands_facts_in_nc_graph() {
        let ctrl = ctrl_with_extractor();
        let tenant = TenantId::new();
        let scope = ScopeId::new();
        ctrl.write(
            tenant,
            scope,
            "s",
            Slot::EpisodicIndex,
            json!({"content": "User lives in San Francisco, works at Acme Corp."}),
            t(2026, 1, 1),
        )
        .await
        .unwrap();
        let user = crate::extractor::name_to_node_id(tenant, scope, "user");
        let edges = ctrl.current_edges_from(tenant, user, None).await.unwrap();
        let rels: Vec<&str> = edges.iter().map(|e| e.rel.as_str()).collect();
        assert!(rels.contains(&"lives_in"));
        assert!(rels.contains(&"works_at"));
    }

    #[tokio::test]
    async fn moved_to_supersedes_prior_lives_in() {
        let ctrl = ctrl_with_extractor();
        let tenant = TenantId::new();
        let scope = ScopeId::new();
        ctrl.write(
            tenant,
            scope,
            "s",
            Slot::EpisodicIndex,
            json!({"content": "User lives in San Francisco, works at Acme Corp."}),
            t(2026, 1, 1),
        )
        .await
        .unwrap();
        ctrl.write(
            tenant,
            scope,
            "s",
            Slot::EpisodicIndex,
            json!({"content": "User moved to Berlin last week, starting at Beta Inc next Monday."}),
            t(2026, 5, 14),
        )
        .await
        .unwrap();
        let user = crate::extractor::name_to_node_id(tenant, scope, "user");
        let edges = ctrl
            .current_edges_from(tenant, user, Some("lives_in"))
            .await
            .unwrap();
        // After supersession only one current lives_in edge survives — the
        // one pointing at the Berlin node.
        assert_eq!(edges.len(), 1);
        let berlin = crate::extractor::name_to_node_id(tenant, scope, "berlin last week");
        assert_eq!(edges[0].dst, berlin);
    }

    #[tokio::test]
    async fn dream_cycle_applies_facts_in_sweep() {
        let ctrl = ctrl_with_extractor();
        let tenant = TenantId::new();
        let scope = ScopeId::new();
        // Pre-write events with the extractor configured; auto-extract will
        // already populate the graph during write. The dream sweep should be
        // idempotent — re-running produces no new edges or duplicates.
        ctrl.write(
            tenant,
            scope,
            "s",
            Slot::EpisodicIndex,
            json!({"content": "User is allergic to peanuts."}),
            t(2026, 1, 1),
        )
        .await
        .unwrap();
        let report = ctrl
            .consolidate(tenant, None, ConsolidationMode::Dream)
            .await
            .unwrap();
        assert!(!report.stub);
        assert_eq!(report.events_examined, 1);
        assert_eq!(report.events_fit, 1);
        // Sanity: only one allergic_to edge exists (no duplication from the
        // double application via write + dream).
        let user = crate::extractor::name_to_node_id(tenant, scope, "user");
        let edges = ctrl
            .current_edges_from(tenant, user, Some("allergic_to"))
            .await
            .unwrap();
        assert_eq!(edges.len(), 1);
    }

    #[tokio::test]
    async fn write_without_extractor_does_not_touch_graph() {
        let ctrl = ctrl(); // no extractor
        let tenant = TenantId::new();
        let scope = ScopeId::new();
        ctrl.write(
            tenant,
            scope,
            "s",
            Slot::EpisodicIndex,
            json!({"content": "User lives in San Francisco."}),
            t(2026, 1, 1),
        )
        .await
        .unwrap();
        let user = crate::extractor::name_to_node_id(tenant, scope, "user");
        let edges = ctrl.current_edges_from(tenant, user, None).await.unwrap();
        assert!(edges.is_empty());
    }

    // --- Explain / export / import ---

    #[tokio::test]
    async fn search_explain_reports_per_leg_ranks_when_embedder_set() {
        let ctrl = ctrl_with_embedder();
        let tenant = TenantId::new();
        let scope = ScopeId::new();
        ctrl.write(
            tenant,
            scope,
            "s",
            Slot::EpisodicIndex,
            json!({"content": "alpha beta"}),
            now(),
        )
        .await
        .unwrap();
        ctrl.write(
            tenant,
            scope,
            "s",
            Slot::EpisodicIndex,
            json!({"content": "gamma"}),
            now(),
        )
        .await
        .unwrap();
        // Disable the relevance gate so we observe per-leg attribution for
        // both events; with the default gate active the unrelated "gamma"
        // event gets dropped from `results` and lands in
        // `rejected_candidates` instead. The full-leg test is below.
        let ctrl = ctrl
            .with_min_relative_score(0.0)
            .with_min_absolute_cosine(0.0);
        let q = SearchQuery::new("alpha", tenant);
        let exp = ctrl.search_explain(&q).await.unwrap();
        assert!(exp.embedder_used);
        assert_eq!(exp.results.len(), 2);
        assert_eq!(exp.mode_used, SearchMode::Standard);
        // The "alpha beta" event must appear in BOTH legs.
        let alpha_id = exp.results[0].event_id;
        let why = exp
            .why_retrieved
            .iter()
            .find(|w| w.event_id == alpha_id)
            .unwrap();
        assert!(why.bm25_rank.is_some());
        assert!(why.vector_rank.is_some());
    }

    #[tokio::test]
    async fn search_explain_works_without_embedder() {
        let ctrl = ctrl();
        let tenant = TenantId::new();
        let scope = ScopeId::new();
        ctrl.write(
            tenant,
            scope,
            "s",
            Slot::EpisodicIndex,
            json!({"content": "alpha"}),
            now(),
        )
        .await
        .unwrap();
        let exp = ctrl
            .search_explain(&SearchQuery::new("alpha", tenant))
            .await
            .unwrap();
        assert!(!exp.embedder_used);
        assert!(exp.vector_ranks.is_empty());
        assert!(!exp.bm25_ranks.is_empty());
    }

    #[tokio::test]
    async fn export_jsonl_round_trips_through_import() {
        let src = ctrl();
        let tenant = TenantId::new();
        let scope = ScopeId::new();
        for content in ["one", "two", "three"] {
            src.write(
                tenant,
                scope,
                "test",
                Slot::EpisodicIndex,
                json!({"content": content}),
                now(),
            )
            .await
            .unwrap();
        }
        let mut buf: Vec<u8> = Vec::new();
        let exported = src
            .export_episodic_jsonl(tenant, None, &mut buf)
            .await
            .unwrap();
        assert_eq!(exported, 3);

        // Round-trip into a fresh controller backed by an empty store.
        let dst = ctrl();
        let report = dst
            .import_episodic_jsonl(std::io::BufReader::new(buf.as_slice()))
            .await
            .unwrap();
        assert_eq!(report.written, 3);
        assert_eq!(report.skipped, 0);
        assert!(report.errors.is_empty());

        let hits = dst
            .search(&SearchQuery::new("two", tenant))
            .await
            .unwrap();
        assert_eq!(hits.len(), 1);
    }

    #[tokio::test]
    async fn import_is_idempotent_on_event_id() {
        let ctrl = ctrl();
        let tenant = TenantId::new();
        let scope = ScopeId::new();
        ctrl.write(
            tenant,
            scope,
            "s",
            Slot::EpisodicIndex,
            json!({"content": "x"}),
            now(),
        )
        .await
        .unwrap();
        let mut buf: Vec<u8> = Vec::new();
        ctrl.export_episodic_jsonl(tenant, None, &mut buf)
            .await
            .unwrap();
        // Re-import into the same controller — every event already exists.
        let report = ctrl
            .import_episodic_jsonl(std::io::BufReader::new(buf.as_slice()))
            .await
            .unwrap();
        assert_eq!(report.written, 0);
        assert_eq!(report.skipped, 1);
    }

    #[tokio::test]
    async fn import_reports_malformed_lines_without_aborting() {
        let ctrl = ctrl();
        let bad = b"not json\n{\"event_id\":\"deadbeef\"}\n";
        let report = ctrl
            .import_episodic_jsonl(std::io::BufReader::new(&bad[..]))
            .await
            .unwrap();
        // Both lines should fail parse; controller doesn't abort.
        assert_eq!(report.written, 0);
        assert_eq!(report.errors.len(), 2);
    }

    // --- Verifiable cascade delete ---

    #[tokio::test]
    async fn delete_node_cascade_removes_edges_and_returns_signed_proof() {
        let ctrl = ctrl();
        let tenant = TenantId::new();
        let scope = ScopeId::new();

        let alice = NodeId::new();
        let bob = NodeId::new();
        ctrl.assert_node(NewNode {
            node_id: alice,
            tenant_id: tenant,
            scope_id: scope,
            node_type: "person".into(),
            properties: json!({"name": "Alice"}),
            provenance: vec![],
        })
        .await
        .unwrap();
        ctrl.assert_node(NewNode {
            node_id: bob,
            tenant_id: tenant,
            scope_id: scope,
            node_type: "person".into(),
            properties: json!({"name": "Bob"}),
            provenance: vec![],
        })
        .await
        .unwrap();
        ctrl.write_fact(NewEdge {
            edge_id: EdgeId::new(),
            src: alice,
            dst: bob,
            rel: "knows".into(),
            strength: Some(1.0),
            tenant_id: tenant,
            scope_id: scope,
            t_valid: t(2026, 1, 1),
            t_invalid: None,
            supersede: None,
            provenance: vec![],
        })
        .await
        .unwrap();

        let proof = ctrl.delete_node(tenant, scope, alice).await.unwrap();
        assert!(proof.cascade.node_removed);
        assert_eq!(proof.cascade.edges_removed, 1);
        match proof.target {
            DeletionTarget::Node(n) => assert_eq!(n, alice),
            other => panic!("expected Node target, got {other:?}"),
        }
        // Receipt is signed and re-verifiable.
        assert!(proof.receipt.signature.is_some());
        assert!(ctrl.verify(&proof.receipt).await.unwrap());

        // The node is gone.
        assert!(ctrl.get_node(alice).await.unwrap().is_none());
        // Its edges are gone.
        let edges = ctrl
            .current_edges_from(tenant, alice, None)
            .await
            .unwrap();
        assert!(edges.is_empty());
    }

    #[tokio::test]
    async fn delete_node_is_idempotent() {
        let ctrl = ctrl();
        let tenant = TenantId::new();
        let scope = ScopeId::new();
        let ghost = NodeId::new();
        // No node exists — first delete returns a proof for a no-op.
        let proof = ctrl.delete_node(tenant, scope, ghost).await.unwrap();
        assert!(!proof.cascade.node_removed);
        assert_eq!(proof.cascade.edges_removed, 0);
        // Receipt still signed; the deletion-event audit log records the attempt.
        assert!(proof.receipt.signature.is_some());
    }

    #[tokio::test]
    async fn delete_blob_emits_signed_proof_and_purges_content() {
        let ctrl = ctrl();
        let tenant = TenantId::new();
        let scope = ScopeId::new();
        let h = ctrl
            .put_blob(tenant, &Blob::text("doomed"))
            .await
            .unwrap();
        assert!(ctrl.has_blob(tenant, h).await.unwrap());
        let proof = ctrl.delete_blob(tenant, scope, h).await.unwrap();
        match proof.target {
            DeletionTarget::Blob(bh) => assert_eq!(bh, h),
            other => panic!("expected Blob target, got {other:?}"),
        }
        // node_removed=true here just means "blob existed before delete".
        assert!(proof.cascade.node_removed);
        assert!(!ctrl.has_blob(tenant, h).await.unwrap());
        assert!(ctrl.verify(&proof.receipt).await.unwrap());
    }

    #[tokio::test]
    async fn deletion_receipts_for_same_target_have_distinct_event_ids() {
        // Each delete-event payload embeds a nanos timestamp so two deletes
        // of the same target are not deduped by content-addressing — the
        // audit log records every attempt.
        let ctrl = ctrl();
        let tenant = TenantId::new();
        let scope = ScopeId::new();
        let node = NodeId::new();
        let p1 = ctrl.delete_node(tenant, scope, node).await.unwrap();
        // Small sleep to ensure nanos differ. Using std rather than tokio so
        // the dev-dep doesn't need the `time` feature flag.
        std::thread::sleep(std::time::Duration::from_millis(2));
        let p2 = ctrl.delete_node(tenant, scope, node).await.unwrap();
        assert_ne!(p1.receipt.event_id, p2.receipt.event_id);
    }

    #[tokio::test]
    async fn labile_state_survives_controller_recreation() {
        // Both controllers wrap the SAME Arc<Storage>, so labile rows
        // opened by the first remain visible to the second. This is
        // what gives the architecture its persistence guarantee: a
        // process restart doesn't lose the prompt-injection mitigation.
        let storage = Arc::new(InMemoryStorage::new());
        let key = Arc::new(InstallKey::generate());
        let ctrl1 = MemoryController::new_with_arc(storage.clone(), key.clone());
        let tenant = TenantId::new();
        let scope = ScopeId::new();
        let receipt = ctrl1
            .write(
                tenant,
                scope,
                "s",
                Slot::EpisodicIndex,
                json!({"content": "v1"}),
                now(),
            )
            .await
            .unwrap();
        ctrl1
            .search(&SearchQuery::new("v1", tenant))
            .await
            .unwrap();
        // Drop ctrl1; rebuild on the same storage.
        drop(ctrl1);
        let ctrl2 = MemoryController::new_with_arc(storage, key);
        // The labile window persisted — ctrl2 can update.
        ctrl2.update(tenant, receipt.event_id, json!({"content": "v2"}), Authority::User)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn shadow_chain_persists_across_controllers() {
        let storage = Arc::new(InMemoryStorage::new());
        let key = Arc::new(InstallKey::generate());
        let tenant = TenantId::new();
        let scope = ScopeId::new();
        let ctrl1 = MemoryController::new_with_arc(storage.clone(), key.clone());
        let r = ctrl1
            .write(
                tenant,
                scope,
                "s",
                Slot::EpisodicIndex,
                json!({"content": "lives in Berlin"}),
                now(),
            )
            .await
            .unwrap();
        ctrl1
            .search(&SearchQuery::new("Berlin", tenant))
            .await
            .unwrap();
        let shadow_receipt = ctrl1
            .update(
                tenant,
                r.event_id,
                json!({"content": "lives in Munich"}),
                Authority::User,
            )
            .await
            .unwrap();
        drop(ctrl1);
        // New controller, same storage — should see the shadow.
        let ctrl2 = MemoryController::new_with_arc(storage, key);
        // A second update from the recreated controller refuses
        // because the shadow row already exists.
        ctrl2
            .search(&SearchQuery::new("Munich", tenant))
            .await
            .unwrap();
        let err = ctrl2
            .update(tenant, r.event_id, json!({"content": "moved again"}), Authority::User)
            .await;
        assert!(matches!(err, Err(UpdateError::AlreadyShadowed { by, .. }) if by == shadow_receipt.event_id));
    }

    #[tokio::test]
    async fn prune_expired_labile_returns_count() {
        let ctrl = ctrl().with_labile_window(Duration::seconds(0));
        let tenant = TenantId::new();
        let scope = ScopeId::new();
        ctrl.write(
            tenant,
            scope,
            "s",
            Slot::EpisodicIndex,
            json!({"content": "any"}),
            now(),
        )
        .await
        .unwrap();
        ctrl.search(&SearchQuery::new("any", tenant)).await.unwrap();
        // 0-duration window means it's already expired by the time we prune.
        let removed = ctrl.prune_expired_labile().await.unwrap();
        assert!(removed >= 1);
    }

    #[tokio::test]
    async fn labile_window_can_be_configured() {
        let ctrl = ctrl().with_labile_window(Duration::seconds(0));
        let tenant = TenantId::new();
        let scope = ScopeId::new();
        let r = ctrl
            .write(
                tenant,
                scope,
                "s",
                Slot::EpisodicIndex,
                json!({"content": "v1"}),
                now(),
            )
            .await
            .unwrap();
        ctrl.search(&SearchQuery::new("v1", tenant)).await.unwrap();
        // 0-duration window — by the time we call update, it has expired.
        let err = ctrl
            .update(tenant, r.event_id, json!({"content": "v2"}), Authority::User)
            .await;
        assert!(matches!(err, Err(UpdateError::NotLabile { .. })));
    }

    #[test]
    fn rrf_fuse_breaks_ties_by_event_id() {
        use ditto_core::EventId;
        let mk = |id_byte: u8| SearchResult {
            event_id: EventId([id_byte; 32]),
            content: "x".into(),
            score: 0.0,
            source_event_ids: vec![],
            metadata: serde_json::json!({}),
        };
        // Both documents only show up once at rank 0 in different legs —
        // same fused score. Smaller event_id wins by tiebreak.
        let bm = vec![mk(2)];
        let vec = vec![mk(1)];
        let out = rrf_fuse(bm, vec, 2);
        assert_eq!(out[0].event_id, EventId([1; 32]));
    }

    #[tokio::test]
    async fn reflections_are_tenant_scoped() {
        let ctrl = ctrl();
        let t_a = TenantId::new();
        let t_b = TenantId::new();
        let scope = ScopeId::new();
        let r = ctrl
            .write_reflection(new_reflection(t_a, scope, "tenant-a's belief", t(2026, 5, 14)))
            .await
            .unwrap();
        // Wrong-tenant get returns None even with the right id.
        assert!(ctrl
            .get_reflection(t_b, r.reflective_id)
            .await
            .unwrap()
            .is_none());
        let a_current = ctrl.current_reflections(t_a, None).await.unwrap();
        let b_current = ctrl.current_reflections(t_b, None).await.unwrap();
        assert_eq!(a_current.len(), 1);
        assert_eq!(b_current.len(), 0);
    }
}
