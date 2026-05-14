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

use chrono::{DateTime, Utc};
use serde_json::Value;
use tokio::sync::Mutex;

use ditto_core::{
    Blob, BlobHash, Edge, EdgeId, Event, EventId, InstallKey, NewEdge, NewNode, Node, NodeId,
    Receipt, ScopeId, SchemaVersion, Slot, TenantId, CURRENT_SCHEMA_VERSION,
};

use crate::search::{SearchQuery, SearchResult};
use crate::storage::{Storage, StorageError, StorageResult};

#[derive(Clone, Copy)]
pub enum SigningPolicy {
    /// Sign every receipt. Production default.
    Required,
    /// Skip signing. Test/embedded only.
    Skip,
}

pub struct MemoryController<S: Storage> {
    storage: Arc<S>,
    install_key: Arc<InstallKey>,
    signing: SigningPolicy,
    schema_version: SchemaVersion,
    chain_heads: Mutex<HashMap<(TenantId, String), EventId>>,
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
        }
    }

    pub fn with_signing_policy(mut self, policy: SigningPolicy) -> Self {
        self.signing = policy;
        self
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

    pub async fn search(&self, query: &SearchQuery) -> StorageResult<Vec<SearchResult>> {
        self.storage.search(query).await
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
}
