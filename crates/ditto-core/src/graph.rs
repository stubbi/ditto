//! Bi-temporal NC-graph types.
//!
//! Nodes are immutable stable concept handles in v0. Edges carry all
//! bi-temporal semantics (transaction time + valid time).
//!
//! See [`docs/architecture/memory.md`] for the broader contract. See
//! `migrations/20260514120001_nc_graph.sql` for the schema this maps onto.

use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::Error;
use crate::id::{EventId, ScopeId, TenantId};

/// Logical identity for an NC-graph node. UUID. Stable across edge changes.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct NodeId(pub Uuid);

impl NodeId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for NodeId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for NodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl FromStr for NodeId {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(Uuid::parse_str(s)?))
    }
}

/// Identity for a single edge row. Bi-temporal — multiple edges may share
/// the same (src, rel, dst) at different valid-time windows.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct EdgeId(pub Uuid);

impl EdgeId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for EdgeId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for EdgeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl FromStr for EdgeId {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(Uuid::parse_str(s)?))
    }
}

/// A graph node. Immutable in v0; identified by `node_id`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Node {
    pub node_id: NodeId,
    pub tenant_id: TenantId,
    pub scope_id: ScopeId,
    pub node_type: String,
    pub properties: serde_json::Value,
    pub t_created: DateTime<Utc>,
    pub provenance: Vec<EventId>,
}

/// Input for `insert_node` / `assert_node`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NewNode {
    /// Pre-assigned `NodeId`. Callers control identity so they can build
    /// idempotent upserts and avoid duplicate "Person:Alice" rows.
    pub node_id: NodeId,
    pub tenant_id: TenantId,
    pub scope_id: ScopeId,
    pub node_type: String,
    pub properties: serde_json::Value,
    pub provenance: Vec<EventId>,
}

/// A bi-temporal edge in the graph.
///
/// `t_expired` is set by the storage layer when the edge is superseded by
/// another transaction. `t_invalid` is set when the *fact* stopped being
/// true — this can be retroactively assigned (the system learns at time T
/// that a fact became false at some earlier time T').
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Edge {
    pub edge_id: EdgeId,
    pub src: NodeId,
    pub dst: NodeId,
    pub rel: String,
    pub strength: f32,
    pub tenant_id: TenantId,
    pub scope_id: ScopeId,
    pub t_created: DateTime<Utc>,
    pub t_expired: Option<DateTime<Utc>>,
    pub t_valid: DateTime<Utc>,
    pub t_invalid: Option<DateTime<Utc>>,
    pub provenance: Vec<EventId>,
}

impl Edge {
    /// An edge is "current" (in both transaction and valid time, as of now)
    /// iff neither `t_expired` nor `t_invalid` is set.
    pub fn is_current(&self) -> bool {
        self.t_expired.is_none() && self.t_invalid.is_none()
    }

    /// Was this edge valid at point in time `t` (valid-time query)?
    /// Ignores transaction time — for "what was the database state as of
    /// the latest transaction" semantics, also gate on `t_expired`.
    pub fn is_valid_at(&self, t: DateTime<Utc>) -> bool {
        self.t_valid <= t && self.t_invalid.is_none_or_after(t)
    }
}

trait OptDateTimeExt {
    fn is_none_or_after(&self, t: DateTime<Utc>) -> bool;
}

impl OptDateTimeExt for Option<DateTime<Utc>> {
    fn is_none_or_after(&self, t: DateTime<Utc>) -> bool {
        match self {
            None => true,
            Some(s) => *s > t,
        }
    }
}

/// Supersession policy for `insert_edge`.
///
/// When an existing edge contradicts a newly inserted one, the prior edge
/// gets `t_invalid = new.t_valid` (the world stopped being one way as the
/// new fact's validity began) and `t_expired = now()` (the database has
/// updated its mind). The new edge is inserted alongside.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SupersedePolicy {
    /// Invalidate every current edge from `src` with the same relation.
    /// Use for functional relations (lives_in, employed_by, married_to).
    AnyWithSameRelation,
    /// Invalidate only current edges from `src` to the same `dst` with the
    /// same relation. Use for "the same fact, restated with a new time".
    SameSrcRelDst,
}

/// Input for `insert_edge`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NewEdge {
    /// Pre-assigned `EdgeId` so callers can build idempotent writes.
    pub edge_id: EdgeId,
    pub src: NodeId,
    pub dst: NodeId,
    pub rel: String,
    pub strength: Option<f32>,
    pub tenant_id: TenantId,
    pub scope_id: ScopeId,
    /// Valid-time start. The new fact is considered true from this instant.
    pub t_valid: DateTime<Utc>,
    /// Valid-time end, if known.
    pub t_invalid: Option<DateTime<Utc>>,
    pub provenance: Vec<EventId>,
    /// If `Some`, the storage layer will invalidate matching prior edges
    /// at this new edge's `t_valid` before inserting the new row, all in
    /// the same transaction.
    pub supersede: Option<SupersedePolicy>,
}
