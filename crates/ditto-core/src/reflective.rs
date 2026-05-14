//! The Reflective slot: consolidator-derived higher-order representations.
//!
//! Reflective records are *derived* — produced by the consolidator (dream
//! cycle) from clusters of raw episodic events. They're the "the user
//! strongly prefers X" / "deploys tend to fail on Fridays" tier of memory.
//! Distinct from NC-graph edges (specific typed facts) by being free-form
//! text + confidence + source provenance.
//!
//! Bi-temporal like nc_edge: `t_valid` (when the reflection started being
//! true), `t_invalid` (when it stopped). New reflections that contradict
//! existing ones cause the prior to be invalidated rather than overwritten —
//! the audit trail of how the agent's beliefs evolved is preserved.
//!
//! `source_event_ids` cites the episodic events that the consolidator
//! considered, and `consolidation_receipt` cites the receipt of the
//! consolidator's own commit (so we can verify the reflection was produced
//! by a recognised consolidation pass, not injected after the fact).

use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::Error;
use crate::id::{EventId, ScopeId, TenantId};

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct ReflectiveId(pub Uuid);

impl ReflectiveId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for ReflectiveId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for ReflectiveId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl FromStr for ReflectiveId {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(Uuid::parse_str(s)?))
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Reflective {
    pub reflective_id: ReflectiveId,
    pub tenant_id: TenantId,
    pub scope_id: ScopeId,
    pub content: String,
    /// Confidence in [0.0, 1.0]. Storage layer clamps out-of-range inputs.
    pub confidence: f32,
    /// Episodic events the consolidator considered when producing this
    /// reflection. Empty when a reflection is user-asserted rather than
    /// consolidator-derived.
    pub source_event_ids: Vec<EventId>,
    /// Event id of the consolidator's commit receipt. `None` when the
    /// reflection is user-asserted rather than derived.
    pub consolidation_receipt: Option<EventId>,
    pub t_created: DateTime<Utc>,
    pub t_expired: Option<DateTime<Utc>>,
    pub t_valid: DateTime<Utc>,
    pub t_invalid: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug)]
pub struct NewReflective {
    pub reflective_id: ReflectiveId,
    pub tenant_id: TenantId,
    pub scope_id: ScopeId,
    pub content: String,
    pub confidence: f32,
    pub source_event_ids: Vec<EventId>,
    pub consolidation_receipt: Option<EventId>,
    /// When this reflection becomes true. Often the timestamp of the latest
    /// supporting episodic event; not necessarily `now()`.
    pub t_valid: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reflective_id_round_trips_through_str() {
        let a = ReflectiveId::new();
        let b = ReflectiveId::from_str(&a.to_string()).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn new_reflective_id_is_unique() {
        let a = ReflectiveId::new();
        let b = ReflectiveId::new();
        assert_ne!(a, b);
    }
}
