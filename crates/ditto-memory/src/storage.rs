//! The `Storage` trait. Implementations: `InMemoryStorage` (this crate) and
//! `PostgresStorage` (in `ditto-storage-postgres`).
//!
//! Storage is the durable side of the controller. It must be linearizable on
//! `write` per `(tenant_id, source_id)` — the controller does not provide that
//! guarantee on its own. In practice this means: write inside a transaction,
//! take an advisory lock or unique-on-event_id, and let the database reject
//! conflicts.

use async_trait::async_trait;
use chrono::{DateTime, Utc};

use ditto_core::{
    Blob, BlobHash, Edge, EdgeId, Event, EventId, NewEdge, NewNode, NewReflective, NewSkill, Node,
    NodeId, Receipt, Reflective, ReflectiveId, ScopeId, Skill, SkillId, SkillStatus, TenantId,
};

use crate::search::{SearchQuery, SearchResult, VectorSearchQuery};

#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("storage error: {0}")]
    Other(String),

    #[error("event not found: {0}")]
    NotFound(EventId),

    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("ditto-core: {0}")]
    Core(#[from] ditto_core::Error),
}

pub type StorageResult<T> = Result<T, StorageError>;

#[async_trait]
pub trait Storage: Send + Sync {
    /// Commit an event and its signed receipt atomically.
    ///
    /// MUST be idempotent on `event.event_id`: a second write of the same
    /// event_id returns the existing receipt without modifying state.
    async fn commit(&self, event: &Event, receipt: &Receipt) -> StorageResult<()>;

    /// Fetch the receipt for an event_id, if present.
    async fn get_receipt(&self, event_id: &EventId) -> StorageResult<Option<Receipt>>;

    /// Fetch the original event (for receipt verification).
    async fn get_event(&self, event_id: &EventId) -> StorageResult<Option<Event>>;

    /// Search the tenant's memory.
    async fn search(&self, query: &SearchQuery) -> StorageResult<Vec<SearchResult>>;

    /// Wipe all data for `tenant_id`. Used between benchmark runs.
    async fn reset(&self, tenant_id: TenantId) -> StorageResult<()>;

    // --- NC-graph ---

    /// Insert a node. Errors if `node_id` already exists.
    async fn insert_node(&self, node: NewNode) -> StorageResult<Node>;

    /// Insert a node if missing; otherwise return the existing one. The
    /// existing row is returned even if its `properties` differ — callers
    /// can detect drift via the returned record.
    async fn assert_node(&self, node: NewNode) -> StorageResult<Node>;

    async fn get_node(&self, node_id: NodeId) -> StorageResult<Option<Node>>;

    /// Insert an edge. If `new_edge.supersede` is set, this MUST run
    /// atomically with the corresponding invalidations of prior matching
    /// edges (same transaction).
    async fn insert_edge(&self, new_edge: NewEdge) -> StorageResult<Edge>;

    async fn get_edge(&self, edge_id: EdgeId) -> StorageResult<Option<Edge>>;

    /// Current outgoing edges from `src`, optionally filtered by relation.
    /// "Current" means `t_expired IS NULL AND t_invalid IS NULL`.
    async fn current_edges_from(
        &self,
        tenant_id: TenantId,
        src: NodeId,
        rel: Option<&str>,
    ) -> StorageResult<Vec<Edge>>;

    /// Current incoming edges to `dst`, optionally filtered by relation.
    async fn current_edges_to(
        &self,
        tenant_id: TenantId,
        dst: NodeId,
        rel: Option<&str>,
    ) -> StorageResult<Vec<Edge>>;

    /// Edges from `src` that were valid at `t` (valid-time query).
    /// Ignores `t_expired` — call site should gate further if it cares
    /// about transaction-time snapshots.
    async fn edges_from_at(
        &self,
        tenant_id: TenantId,
        src: NodeId,
        t: DateTime<Utc>,
    ) -> StorageResult<Vec<Edge>>;

    /// Set `t_invalid` on an edge. The edge stays — it is now known to have
    /// stopped being true at `t_invalid`. Idempotent on equal values.
    async fn invalidate_edge(
        &self,
        edge_id: EdgeId,
        t_invalid: DateTime<Utc>,
    ) -> StorageResult<()>;

    /// All nodes in the tenant, optionally filtered to one scope. Used by
    /// the NC-doc renderer to enumerate pages.
    async fn list_nodes(
        &self,
        tenant_id: TenantId,
        scope_id: Option<ScopeId>,
    ) -> StorageResult<Vec<Node>>;

    /// All outgoing edges from `src` (including invalidated and expired),
    /// optionally filtered by relation. Used by the NC-doc renderer for the
    /// historical-facts section.
    async fn edges_from_all_time(
        &self,
        tenant_id: TenantId,
        src: NodeId,
        rel: Option<&str>,
    ) -> StorageResult<Vec<Edge>>;

    /// All incoming edges to `dst` (including invalidated and expired).
    async fn edges_to_all_time(
        &self,
        tenant_id: TenantId,
        dst: NodeId,
        rel: Option<&str>,
    ) -> StorageResult<Vec<Edge>>;

    // --- Blob store (CAS) ---

    /// Idempotently store a blob. Returns the SHA-256 of the blob bytes. A
    /// second `put_blob` with identical bytes for the same tenant is a no-op
    /// and returns the same hash.
    ///
    /// Tenancy: blob storage is partitioned by `tenant_id` even though the
    /// hash is intrinsic — the same bytes can be persisted independently by
    /// two tenants, and a delete by one tenant must not affect the other.
    async fn put_blob(&self, tenant_id: TenantId, blob: &Blob) -> StorageResult<BlobHash>;

    /// Fetch a blob by its hash. Returns `None` if the tenant has not stored
    /// this blob (even if another tenant has).
    async fn get_blob(&self, tenant_id: TenantId, hash: BlobHash) -> StorageResult<Option<Blob>>;

    /// Cheap existence check. Avoids a payload read when the caller only
    /// needs to know whether to write.
    async fn has_blob(&self, tenant_id: TenantId, hash: BlobHash) -> StorageResult<bool>;

    // --- Procedural (skills) ---

    /// Register a skill. Idempotent on `(tenant_id, skill_id)` when the
    /// `version` matches the existing row; errors if a different version is
    /// registered under the same id (use `update_skill_version` to migrate).
    /// New skills land with `status = active`, `last_used = None`,
    /// `tests_pass = None`.
    async fn register_skill(&self, skill: NewSkill) -> StorageResult<Skill>;

    async fn get_skill(
        &self,
        tenant_id: TenantId,
        skill_id: &SkillId,
    ) -> StorageResult<Option<Skill>>;

    /// List skills for a tenant. `status_filter = Some(s)` restricts the
    /// result; `None` returns every row regardless of lifecycle stage.
    async fn list_skills(
        &self,
        tenant_id: TenantId,
        status_filter: Option<SkillStatus>,
    ) -> StorageResult<Vec<Skill>>;

    /// Bump `last_used`. Metabolism rules consult this to decide deprecation
    /// in the dream cycle.
    async fn mark_skill_used(
        &self,
        tenant_id: TenantId,
        skill_id: &SkillId,
        at: DateTime<Utc>,
    ) -> StorageResult<()>;

    /// Record a test-pass rate in [0.0, 1.0]. Out-of-range values are clamped.
    async fn set_skill_tests_pass(
        &self,
        tenant_id: TenantId,
        skill_id: &SkillId,
        pass: f32,
    ) -> StorageResult<()>;

    /// Transition lifecycle state: active → deprecated → archived. Backwards
    /// transitions are allowed (a deprecated skill can be reactivated).
    async fn set_skill_status(
        &self,
        tenant_id: TenantId,
        skill_id: &SkillId,
        status: SkillStatus,
    ) -> StorageResult<()>;

    // --- Reflective ---

    /// Insert a reflection. Errors if `reflective_id` already exists.
    /// `t_created` is set by the backend at insert time. Caller may set
    /// `t_valid` to a past timestamp (when this insight became true).
    async fn insert_reflective(&self, new: NewReflective) -> StorageResult<Reflective>;

    async fn get_reflective(
        &self,
        tenant_id: TenantId,
        reflective_id: ReflectiveId,
    ) -> StorageResult<Option<Reflective>>;

    /// Current reflections — `t_expired IS NULL AND t_invalid IS NULL`.
    /// Filtered to one scope if `scope_id` is `Some`.
    async fn current_reflective(
        &self,
        tenant_id: TenantId,
        scope_id: Option<ScopeId>,
    ) -> StorageResult<Vec<Reflective>>;

    /// All reflections regardless of bi-temporal state. Used by the NC-doc
    /// renderer for the historical-reflections section and by export.
    async fn list_reflective_all_time(
        &self,
        tenant_id: TenantId,
        scope_id: Option<ScopeId>,
    ) -> StorageResult<Vec<Reflective>>;

    /// Set `t_invalid` on a reflection. Idempotent on equal values.
    async fn invalidate_reflective(
        &self,
        reflective_id: ReflectiveId,
        t_invalid: DateTime<Utc>,
    ) -> StorageResult<()>;

    // --- Dense retrieval ---

    /// Store an embedding vector for an event. Idempotent on `event_id`;
    /// a second `put_embedding` for the same event overwrites (embedders
    /// can be swapped without rewriting events).
    async fn put_embedding(
        &self,
        tenant_id: TenantId,
        event_id: EventId,
        embedding: &[f32],
    ) -> StorageResult<()>;

    /// Vector search. Returns SearchResults shaped identically to `search`
    /// so RRF fusion in the controller can treat both legs symmetrically.
    /// `score` is the cosine similarity in [-1.0, 1.0]; backends that
    /// compute distance instead must convert (1 - distance).
    async fn search_vector(&self, query: &VectorSearchQuery) -> StorageResult<Vec<SearchResult>>;

    // --- Verifiable deletion ---

    /// Delete a node and all its edges (incoming + outgoing, current and
    /// historical) in one atomic transaction. Idempotent: deleting a node
    /// that no longer exists returns `cascade.node_removed = false` with
    /// `edges_removed = 0`, not an error — this lets retries be safe.
    ///
    /// Returns the count of records removed alongside, which the controller
    /// folds into the signed DeletionProof payload.
    async fn delete_node_cascade(
        &self,
        tenant_id: TenantId,
        node_id: NodeId,
    ) -> StorageResult<CascadeReport>;

    /// Delete a blob from the tenant's CAS. Returns true when the blob
    /// existed and was removed, false when it was already absent.
    async fn delete_blob(&self, tenant_id: TenantId, hash: BlobHash) -> StorageResult<bool>;

    /// Enumerate episodic events for a tenant, oldest first. Used by export
    /// and by replay paths. `limit = None` returns all events; backends
    /// should still chunk internally to bound memory.
    async fn list_episodic(
        &self,
        tenant_id: TenantId,
        scope_id: Option<ScopeId>,
        limit: Option<usize>,
    ) -> StorageResult<Vec<Event>>;
}

/// Side effects of a cascade delete — folded into the DeletionProof payload
/// so an auditor can verify "deleting node X removed N edges, M reflections".
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct CascadeReport {
    pub node_removed: bool,
    pub edges_removed: u32,
}
