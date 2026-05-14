//! Reference in-memory `Storage` implementation.
//!
//! Not a production backend. Used for:
//! - Unit testing the controller without a database
//! - The Python eval harness's `stub` backend equivalent — control floor for
//!   substring scanning
//! - A placeholder for the future SQLite embedded mode (same shape, durable
//!   substrate)
//!
//! Retrieval is naive substring matching with a recency tiebreak. The point is
//! to validate the orchestration, not the retrieval quality.

use std::collections::HashMap;
use std::sync::Mutex;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde_json::json;

use ditto_core::{
    Blob, BlobHash, Edge, EdgeId, Event, EventId, NewEdge, NewNode, NewReflective, NewSkill, Node,
    NodeId, Receipt, Reflective, ReflectiveId, ScopeId, Skill, SkillId, SkillStatus, TenantId,
};

use crate::embedder::cosine;
use crate::search::{SearchQuery, SearchResult, VectorSearchQuery};
use crate::storage::{CascadeReport, Storage, StorageError, StorageResult};

#[derive(Default)]
pub struct InMemoryStorage {
    inner: Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
    /// tenant_id -> append-only list of events
    events: HashMap<TenantId, Vec<Event>>,
    /// event_id -> receipt
    receipts: HashMap<EventId, Receipt>,
    /// node_id -> node (nodes are immutable in v0)
    nodes: HashMap<NodeId, Node>,
    /// edge_id -> edge (versioned via bi-temporal cols on the value)
    edges: HashMap<EdgeId, Edge>,
    /// (tenant_id, blob_hash) -> blob. Per-tenant CAS — same bytes for two
    /// tenants are stored twice so deletes can't leak across the isolation
    /// boundary.
    blobs: HashMap<(TenantId, BlobHash), Blob>,
    /// (tenant_id, skill_id) -> Skill. Procedural index.
    skills: HashMap<(TenantId, SkillId), Skill>,
    /// reflective_id -> Reflective (versioned via bi-temporal cols).
    reflective: HashMap<ReflectiveId, Reflective>,
    /// (tenant_id, event_id) -> embedding. Tenant-scoped so a tenant reset
    /// purges its vectors alongside the rest.
    embeddings: HashMap<(TenantId, EventId), Vec<f32>>,
}

impl InMemoryStorage {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl Storage for InMemoryStorage {
    async fn commit(&self, event: &Event, receipt: &Receipt) -> StorageResult<()> {
        let mut inner = self.inner.lock().unwrap();
        // idempotent on event_id
        if inner.receipts.contains_key(&event.event_id) {
            return Ok(());
        }
        inner
            .events
            .entry(event.tenant_id)
            .or_default()
            .push(event.clone());
        inner.receipts.insert(event.event_id, receipt.clone());
        Ok(())
    }

    async fn get_receipt(&self, event_id: &EventId) -> StorageResult<Option<Receipt>> {
        Ok(self.inner.lock().unwrap().receipts.get(event_id).cloned())
    }

    async fn get_event(&self, event_id: &EventId) -> StorageResult<Option<Event>> {
        let inner = self.inner.lock().unwrap();
        for events in inner.events.values() {
            if let Some(e) = events.iter().find(|e| &e.event_id == event_id) {
                return Ok(Some(e.clone()));
            }
        }
        Ok(None)
    }

    async fn search(&self, query: &SearchQuery) -> StorageResult<Vec<SearchResult>> {
        let inner = self.inner.lock().unwrap();
        let events = match inner.events.get(&query.tenant_id) {
            Some(v) => v,
            None => return Ok(Vec::new()),
        };
        let q = query.query.to_lowercase();
        let mut scored: Vec<(f32, &Event)> = Vec::new();
        for e in events {
            if let Some(sources) = &query.sources {
                if !sources.contains(&e.source_id) {
                    continue;
                }
            }
            if let Some(scope_id) = query.scope_id {
                if e.scope_id != scope_id {
                    continue;
                }
            }
            let content = render(e);
            let hits = content.to_lowercase().matches(&q).count();
            if hits == 0 {
                continue;
            }
            let recency_tiebreak = e.timestamp.timestamp() as f32 / 1e12;
            scored.push((hits as f32 + recency_tiebreak, e));
        }
        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        let out = scored
            .into_iter()
            .take(query.k)
            .map(|(score, e)| SearchResult {
                event_id: e.event_id,
                content: render(e),
                score,
                source_event_ids: vec![e.event_id],
                metadata: json!({
                    "source_id": e.source_id,
                    "timestamp": e.timestamp,
                    "slot": e.slot,
                }),
            })
            .collect();
        Ok(out)
    }

    async fn reset(&self, tenant_id: TenantId) -> StorageResult<()> {
        let mut inner = self.inner.lock().unwrap();
        if let Some(events) = inner.events.remove(&tenant_id) {
            for e in events {
                inner.receipts.remove(&e.event_id);
            }
        }
        inner.nodes.retain(|_, n| n.tenant_id != tenant_id);
        inner.edges.retain(|_, e| e.tenant_id != tenant_id);
        inner.blobs.retain(|(t, _), _| *t != tenant_id);
        inner.skills.retain(|(t, _), _| *t != tenant_id);
        inner.reflective.retain(|_, r| r.tenant_id != tenant_id);
        inner.embeddings.retain(|(t, _), _| *t != tenant_id);
        Ok(())
    }

    async fn insert_node(&self, node: NewNode) -> StorageResult<Node> {
        let mut inner = self.inner.lock().unwrap();
        if inner.nodes.contains_key(&node.node_id) {
            return Err(StorageError::Other(format!(
                "node {} already exists",
                node.node_id
            )));
        }
        let n = Node {
            node_id: node.node_id,
            tenant_id: node.tenant_id,
            scope_id: node.scope_id,
            node_type: node.node_type,
            properties: node.properties,
            t_created: Utc::now(),
            provenance: node.provenance,
        };
        inner.nodes.insert(n.node_id, n.clone());
        Ok(n)
    }

    async fn assert_node(&self, node: NewNode) -> StorageResult<Node> {
        let mut inner = self.inner.lock().unwrap();
        if let Some(existing) = inner.nodes.get(&node.node_id) {
            return Ok(existing.clone());
        }
        let n = Node {
            node_id: node.node_id,
            tenant_id: node.tenant_id,
            scope_id: node.scope_id,
            node_type: node.node_type,
            properties: node.properties,
            t_created: Utc::now(),
            provenance: node.provenance,
        };
        inner.nodes.insert(n.node_id, n.clone());
        Ok(n)
    }

    async fn get_node(&self, node_id: NodeId) -> StorageResult<Option<Node>> {
        Ok(self.inner.lock().unwrap().nodes.get(&node_id).cloned())
    }

    async fn insert_edge(&self, new_edge: NewEdge) -> StorageResult<Edge> {
        use ditto_core::SupersedePolicy;
        let mut inner = self.inner.lock().unwrap();
        let now = Utc::now();

        // Supersession runs first: find matching current edges, set t_invalid
        // to the new edge's t_valid and t_expired to now. All in one critical
        // section so it is atomic against concurrent inserts.
        if let Some(policy) = new_edge.supersede {
            for edge in inner.edges.values_mut() {
                if edge.tenant_id != new_edge.tenant_id
                    || edge.src != new_edge.src
                    || edge.rel != new_edge.rel
                    || !edge.is_current()
                {
                    continue;
                }
                let matches = match policy {
                    SupersedePolicy::AnyWithSameRelation => true,
                    SupersedePolicy::SameSrcRelDst => edge.dst == new_edge.dst,
                };
                if matches {
                    edge.t_invalid = Some(new_edge.t_valid);
                    edge.t_expired = Some(now);
                }
            }
        }

        let edge = Edge {
            edge_id: new_edge.edge_id,
            src: new_edge.src,
            dst: new_edge.dst,
            rel: new_edge.rel,
            strength: new_edge.strength.unwrap_or(0.1),
            tenant_id: new_edge.tenant_id,
            scope_id: new_edge.scope_id,
            t_created: now,
            t_expired: None,
            t_valid: new_edge.t_valid,
            t_invalid: new_edge.t_invalid,
            provenance: new_edge.provenance,
        };
        if inner.edges.contains_key(&edge.edge_id) {
            return Err(StorageError::Other(format!(
                "edge {} already exists",
                edge.edge_id
            )));
        }
        inner.edges.insert(edge.edge_id, edge.clone());
        Ok(edge)
    }

    async fn get_edge(&self, edge_id: EdgeId) -> StorageResult<Option<Edge>> {
        Ok(self.inner.lock().unwrap().edges.get(&edge_id).cloned())
    }

    async fn current_edges_from(
        &self,
        tenant_id: TenantId,
        src: NodeId,
        rel: Option<&str>,
    ) -> StorageResult<Vec<Edge>> {
        let inner = self.inner.lock().unwrap();
        Ok(inner
            .edges
            .values()
            .filter(|e| {
                e.tenant_id == tenant_id
                    && e.src == src
                    && e.is_current()
                    && rel.is_none_or(|r| e.rel == r)
            })
            .cloned()
            .collect())
    }

    async fn current_edges_to(
        &self,
        tenant_id: TenantId,
        dst: NodeId,
        rel: Option<&str>,
    ) -> StorageResult<Vec<Edge>> {
        let inner = self.inner.lock().unwrap();
        Ok(inner
            .edges
            .values()
            .filter(|e| {
                e.tenant_id == tenant_id
                    && e.dst == dst
                    && e.is_current()
                    && rel.is_none_or(|r| e.rel == r)
            })
            .cloned()
            .collect())
    }

    async fn edges_from_at(
        &self,
        tenant_id: TenantId,
        src: NodeId,
        t: DateTime<Utc>,
    ) -> StorageResult<Vec<Edge>> {
        let inner = self.inner.lock().unwrap();
        Ok(inner
            .edges
            .values()
            .filter(|e| e.tenant_id == tenant_id && e.src == src && e.is_valid_at(t))
            .cloned()
            .collect())
    }

    async fn invalidate_edge(
        &self,
        edge_id: EdgeId,
        t_invalid: DateTime<Utc>,
    ) -> StorageResult<()> {
        let mut inner = self.inner.lock().unwrap();
        match inner.edges.get_mut(&edge_id) {
            Some(e) => {
                e.t_invalid = Some(t_invalid);
                Ok(())
            }
            None => Err(StorageError::Other(format!(
                "edge {edge_id} not found"
            ))),
        }
    }

    async fn list_nodes(
        &self,
        tenant_id: TenantId,
        scope_id: Option<ScopeId>,
    ) -> StorageResult<Vec<Node>> {
        let inner = self.inner.lock().unwrap();
        let mut out: Vec<Node> = inner
            .nodes
            .values()
            .filter(|n| {
                n.tenant_id == tenant_id && scope_id.is_none_or(|s| n.scope_id == s)
            })
            .cloned()
            .collect();
        out.sort_by_key(|n| n.node_id.0);
        Ok(out)
    }

    async fn edges_from_all_time(
        &self,
        tenant_id: TenantId,
        src: NodeId,
        rel: Option<&str>,
    ) -> StorageResult<Vec<Edge>> {
        let inner = self.inner.lock().unwrap();
        let mut out: Vec<Edge> = inner
            .edges
            .values()
            .filter(|e| {
                e.tenant_id == tenant_id && e.src == src && rel.is_none_or(|r| e.rel == r)
            })
            .cloned()
            .collect();
        out.sort_by(|a, b| {
            a.rel
                .cmp(&b.rel)
                .then(a.t_valid.cmp(&b.t_valid))
                .then(a.dst.0.cmp(&b.dst.0))
        });
        Ok(out)
    }

    async fn edges_to_all_time(
        &self,
        tenant_id: TenantId,
        dst: NodeId,
        rel: Option<&str>,
    ) -> StorageResult<Vec<Edge>> {
        let inner = self.inner.lock().unwrap();
        let mut out: Vec<Edge> = inner
            .edges
            .values()
            .filter(|e| {
                e.tenant_id == tenant_id && e.dst == dst && rel.is_none_or(|r| e.rel == r)
            })
            .cloned()
            .collect();
        out.sort_by(|a, b| {
            a.rel
                .cmp(&b.rel)
                .then(a.t_valid.cmp(&b.t_valid))
                .then(a.src.0.cmp(&b.src.0))
        });
        Ok(out)
    }

    async fn put_blob(&self, tenant_id: TenantId, blob: &Blob) -> StorageResult<BlobHash> {
        let hash = blob.hash();
        let mut inner = self.inner.lock().unwrap();
        // Idempotent on hash. If the same bytes arrive twice the existing
        // record wins — second-writer's content_type does not overwrite.
        inner
            .blobs
            .entry((tenant_id, hash))
            .or_insert_with(|| blob.clone());
        Ok(hash)
    }

    async fn get_blob(&self, tenant_id: TenantId, hash: BlobHash) -> StorageResult<Option<Blob>> {
        let inner = self.inner.lock().unwrap();
        Ok(inner.blobs.get(&(tenant_id, hash)).cloned())
    }

    async fn has_blob(&self, tenant_id: TenantId, hash: BlobHash) -> StorageResult<bool> {
        let inner = self.inner.lock().unwrap();
        Ok(inner.blobs.contains_key(&(tenant_id, hash)))
    }

    async fn register_skill(&self, skill: NewSkill) -> StorageResult<Skill> {
        let key = (skill.tenant_id, skill.skill_id.clone());
        let mut inner = self.inner.lock().unwrap();
        if let Some(existing) = inner.skills.get(&key) {
            if existing.version == skill.version {
                return Ok(existing.clone());
            }
            return Err(StorageError::Other(format!(
                "skill {} already registered with version {} (got {})",
                skill.skill_id, existing.version, skill.version
            )));
        }
        let record = Skill {
            skill_id: skill.skill_id,
            tenant_id: skill.tenant_id,
            scope_id: skill.scope_id,
            version: skill.version,
            path: skill.path,
            last_used: None,
            tests_pass: None,
            status: SkillStatus::Active,
        };
        inner.skills.insert(key, record.clone());
        Ok(record)
    }

    async fn get_skill(
        &self,
        tenant_id: TenantId,
        skill_id: &SkillId,
    ) -> StorageResult<Option<Skill>> {
        let inner = self.inner.lock().unwrap();
        Ok(inner.skills.get(&(tenant_id, skill_id.clone())).cloned())
    }

    async fn list_skills(
        &self,
        tenant_id: TenantId,
        status_filter: Option<SkillStatus>,
    ) -> StorageResult<Vec<Skill>> {
        let inner = self.inner.lock().unwrap();
        let mut out: Vec<Skill> = inner
            .skills
            .iter()
            .filter(|((t, _), _)| *t == tenant_id)
            .filter(|(_, s)| status_filter.is_none_or(|sf| s.status == sf))
            .map(|(_, s)| s.clone())
            .collect();
        out.sort_by(|a, b| a.skill_id.cmp(&b.skill_id));
        Ok(out)
    }

    async fn mark_skill_used(
        &self,
        tenant_id: TenantId,
        skill_id: &SkillId,
        at: DateTime<Utc>,
    ) -> StorageResult<()> {
        let mut inner = self.inner.lock().unwrap();
        let skill = inner
            .skills
            .get_mut(&(tenant_id, skill_id.clone()))
            .ok_or_else(|| StorageError::Other(format!("skill not found: {skill_id}")))?;
        skill.last_used = Some(at);
        Ok(())
    }

    async fn set_skill_tests_pass(
        &self,
        tenant_id: TenantId,
        skill_id: &SkillId,
        pass: f32,
    ) -> StorageResult<()> {
        let mut inner = self.inner.lock().unwrap();
        let skill = inner
            .skills
            .get_mut(&(tenant_id, skill_id.clone()))
            .ok_or_else(|| StorageError::Other(format!("skill not found: {skill_id}")))?;
        skill.tests_pass = Some(pass.clamp(0.0, 1.0));
        Ok(())
    }

    async fn set_skill_status(
        &self,
        tenant_id: TenantId,
        skill_id: &SkillId,
        status: SkillStatus,
    ) -> StorageResult<()> {
        let mut inner = self.inner.lock().unwrap();
        let skill = inner
            .skills
            .get_mut(&(tenant_id, skill_id.clone()))
            .ok_or_else(|| StorageError::Other(format!("skill not found: {skill_id}")))?;
        skill.status = status;
        Ok(())
    }

    async fn insert_reflective(&self, new: NewReflective) -> StorageResult<Reflective> {
        let mut inner = self.inner.lock().unwrap();
        if inner.reflective.contains_key(&new.reflective_id) {
            return Err(StorageError::Other(format!(
                "reflective {} already exists",
                new.reflective_id
            )));
        }
        let record = Reflective {
            reflective_id: new.reflective_id,
            tenant_id: new.tenant_id,
            scope_id: new.scope_id,
            content: new.content,
            confidence: new.confidence.clamp(0.0, 1.0),
            source_event_ids: new.source_event_ids,
            consolidation_receipt: new.consolidation_receipt,
            t_created: Utc::now(),
            t_expired: None,
            t_valid: new.t_valid,
            t_invalid: None,
        };
        inner
            .reflective
            .insert(new.reflective_id, record.clone());
        Ok(record)
    }

    async fn get_reflective(
        &self,
        tenant_id: TenantId,
        reflective_id: ReflectiveId,
    ) -> StorageResult<Option<Reflective>> {
        let inner = self.inner.lock().unwrap();
        Ok(inner
            .reflective
            .get(&reflective_id)
            .filter(|r| r.tenant_id == tenant_id)
            .cloned())
    }

    async fn current_reflective(
        &self,
        tenant_id: TenantId,
        scope_id: Option<ScopeId>,
    ) -> StorageResult<Vec<Reflective>> {
        let inner = self.inner.lock().unwrap();
        let mut out: Vec<Reflective> = inner
            .reflective
            .values()
            .filter(|r| {
                r.tenant_id == tenant_id
                    && r.t_expired.is_none()
                    && r.t_invalid.is_none()
                    && scope_id.is_none_or(|s| r.scope_id == s)
            })
            .cloned()
            .collect();
        // Deterministic order: most-recently-valid first, then id for ties.
        out.sort_by(|a, b| b.t_valid.cmp(&a.t_valid).then(a.reflective_id.0.cmp(&b.reflective_id.0)));
        Ok(out)
    }

    async fn list_reflective_all_time(
        &self,
        tenant_id: TenantId,
        scope_id: Option<ScopeId>,
    ) -> StorageResult<Vec<Reflective>> {
        let inner = self.inner.lock().unwrap();
        let mut out: Vec<Reflective> = inner
            .reflective
            .values()
            .filter(|r| r.tenant_id == tenant_id && scope_id.is_none_or(|s| r.scope_id == s))
            .cloned()
            .collect();
        out.sort_by(|a, b| a.t_valid.cmp(&b.t_valid).then(a.reflective_id.0.cmp(&b.reflective_id.0)));
        Ok(out)
    }

    async fn invalidate_reflective(
        &self,
        reflective_id: ReflectiveId,
        t_invalid: DateTime<Utc>,
    ) -> StorageResult<()> {
        let mut inner = self.inner.lock().unwrap();
        let r = inner
            .reflective
            .get_mut(&reflective_id)
            .ok_or_else(|| StorageError::Other(format!("reflective not found: {reflective_id}")))?;
        if let Some(existing) = r.t_invalid {
            if existing == t_invalid {
                return Ok(());
            }
        }
        r.t_invalid = Some(t_invalid);
        Ok(())
    }

    async fn put_embedding(
        &self,
        tenant_id: TenantId,
        event_id: EventId,
        embedding: &[f32],
    ) -> StorageResult<()> {
        let mut inner = self.inner.lock().unwrap();
        inner
            .embeddings
            .insert((tenant_id, event_id), embedding.to_vec());
        Ok(())
    }

    async fn delete_node_cascade(
        &self,
        tenant_id: TenantId,
        node_id: NodeId,
    ) -> StorageResult<CascadeReport> {
        let mut inner = self.inner.lock().unwrap();
        let node_removed = inner
            .nodes
            .remove(&node_id)
            .filter(|n| n.tenant_id == tenant_id)
            .is_some();
        let mut edges_removed = 0_u32;
        inner.edges.retain(|_, e| {
            let touches_node = e.src == node_id || e.dst == node_id;
            if touches_node && e.tenant_id == tenant_id {
                edges_removed += 1;
                false
            } else {
                true
            }
        });
        Ok(CascadeReport {
            node_removed,
            edges_removed,
        })
    }

    async fn delete_blob(&self, tenant_id: TenantId, hash: BlobHash) -> StorageResult<bool> {
        let mut inner = self.inner.lock().unwrap();
        Ok(inner.blobs.remove(&(tenant_id, hash)).is_some())
    }

    async fn list_episodic(
        &self,
        tenant_id: TenantId,
        scope_id: Option<ScopeId>,
        limit: Option<usize>,
    ) -> StorageResult<Vec<Event>> {
        let inner = self.inner.lock().unwrap();
        let events = match inner.events.get(&tenant_id) {
            Some(v) => v,
            None => return Ok(Vec::new()),
        };
        let mut out: Vec<Event> = events
            .iter()
            .filter(|e| scope_id.is_none_or(|s| e.scope_id == s))
            .cloned()
            .collect();
        out.sort_by(|a, b| a.timestamp.cmp(&b.timestamp).then(a.event_id.0.cmp(&b.event_id.0)));
        if let Some(lim) = limit {
            out.truncate(lim);
        }
        Ok(out)
    }

    async fn search_vector(&self, query: &VectorSearchQuery) -> StorageResult<Vec<SearchResult>> {
        let inner = self.inner.lock().unwrap();
        let events = match inner.events.get(&query.tenant_id) {
            Some(v) => v,
            None => return Ok(Vec::new()),
        };
        let mut scored: Vec<(f32, &Event)> = Vec::new();
        for e in events {
            if let Some(sources) = &query.sources {
                if !sources.contains(&e.source_id) {
                    continue;
                }
            }
            if let Some(scope_id) = query.scope_id {
                if e.scope_id != scope_id {
                    continue;
                }
            }
            let Some(emb) = inner.embeddings.get(&(query.tenant_id, e.event_id)) else {
                continue;
            };
            if emb.len() != query.embedding.len() {
                continue;
            }
            let sim = cosine(&query.embedding, emb);
            scored.push((sim, e));
        }
        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        let out = scored
            .into_iter()
            .take(query.k)
            .map(|(score, e)| SearchResult {
                event_id: e.event_id,
                content: render(e),
                score,
                source_event_ids: vec![e.event_id],
                metadata: json!({
                    "source_id": e.source_id,
                    "timestamp": e.timestamp,
                    "slot": e.slot,
                    "leg": "vector",
                }),
            })
            .collect();
        Ok(out)
    }
}

fn render(event: &Event) -> String {
    if let Some(s) = event.payload.get("content").and_then(|v| v.as_str()) {
        s.to_string()
    } else {
        event.payload.to_string()
    }
}
