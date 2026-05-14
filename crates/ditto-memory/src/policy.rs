//! Operations policy — the Memory-R1 / Mem-α seam.
//!
//! Memory-R1 (Wang et al. 2025) and Mem-α (Park et al. 2025) recast
//! memory management as a sequential decision problem: given an incoming
//! event and the current memory state, a learned policy picks a sequence
//! of *operations* (add, merge, forget, reflect, promote, …) that maximize
//! downstream task reward. The architectural contract is the **operation
//! set + a `Policy` trait surface**; the implementation can be a hand-rolled
//! heuristic, a fine-tuned LLM, or a reinforcement-learned head.
//!
//! v0 ships the operation enum and a no-op `HeuristicPolicy` that returns
//! `[Operation::Add]` for every event — semantically equivalent to the
//! current `write` path. The point of v0 is to **pin the surface** so the
//! controller can grow into calling `policy.decide(ctx)` before commit,
//! and so eval harnesses can swap in alternative policies without touching
//! controller code.
//!
//! Why ten operations and not five (or twenty): the Memory-R1 paper
//! converged on this set after ablating broader and narrower action spaces.
//! We pin the same vocabulary so policies trained against that paper's
//! benchmark transfer here.

use async_trait::async_trait;

use ditto_core::{Event, EventId, NewSkill, NodeId, TenantId};

use crate::controller::Authority;

/// One memory operation. The cardinality matches Memory-R1's action space.
/// Future policies will emit sequences of these per event; v0 commits a
/// single `Add` per write.
#[derive(Clone, Debug)]
pub enum Operation {
    /// Commit the event as-is to episodic. The default action.
    Add,
    /// Reconsolidate an in-window event with corrected content under a
    /// trusted authority. Maps to `MemoryController::update`.
    Update {
        event_id: EventId,
        new_content: String,
        authority: Authority,
    },
    /// Verifiable cascade delete of an NC-graph node and all incident
    /// edges. Maps to `MemoryController::delete_node`.
    Delete { node_id: NodeId },
    /// Synthesize a reflection over a set of source events. Reflections
    /// are stored in the Reflective slot with their own bi-temporal
    /// validity. Maps to `MemoryController::write_reflection`.
    Reflect {
        source_event_ids: Vec<EventId>,
        content: String,
        confidence: f32,
    },
    /// Promote a recurring observation into the Procedural slot as a
    /// reusable skill. Maps to `MemoryController::register_skill`.
    Promote { skill: NewSkill },
    /// Merge two nodes — semantically declare them the same entity. v0
    /// stores the intent; the controller integration is deferred along
    /// with the entity-resolution sweep that detects the duplicate in
    /// the first place.
    Merge { keep: NodeId, drop: NodeId },
    /// Split one node into many — the inverse of `Merge`. Surfaces when
    /// an extracted alias turned out to refer to multiple distinct
    /// entities. v0 stores the intent.
    Split { node_id: NodeId, into: Vec<NodeId> },
    /// Attach an opaque label to an event for downstream filtering.
    /// Closest mechanical analogue is a tag in a tagging system; v0
    /// stores the intent.
    Tag { event_id: EventId, label: String },
    /// Bump salience on an event. Maps to `MemoryController::bump_salience`.
    Strengthen { event_id: EventId, delta: f32 },
    /// Mark an event for accelerated decay or archival. v0 stores the
    /// intent — accelerated decay lands with retrieval-induced
    /// suppression in long sleep.
    Forget { event_id: EventId },
}

/// Read-only context handed to the policy. Crafted to be cheap to clone
/// into LLM prompts later (no `Arc<MemoryController>`, no live storage
/// handles — just the data the policy needs to decide).
#[derive(Clone, Debug)]
pub struct PolicyContext<'a> {
    pub tenant_id: TenantId,
    /// The event being processed.
    pub event: &'a Event,
    /// Recent events from the same tenant, oldest first. The policy may
    /// use these to detect supersession opportunities (same fact stated
    /// again with new content) without hitting storage itself.
    pub recent: &'a [Event],
}

/// The trait policies implement. `decide` returns the operation sequence
/// the controller should apply for `ctx.event`. The empty vector means
/// "drop this event" — useful for adversarial inputs the policy refuses.
#[async_trait]
pub trait Policy: Send + Sync {
    async fn decide(&self, ctx: &PolicyContext<'_>) -> Vec<Operation>;
}

/// Default v0 policy. Every event commits as `Add` — semantically
/// identical to the current pre-policy `write` path. Swap this out for
/// an LLM-driven policy in `ditto-models` once the training pipeline lands.
#[derive(Clone, Copy, Debug, Default)]
pub struct HeuristicPolicy;

#[async_trait]
impl Policy for HeuristicPolicy {
    async fn decide(&self, _ctx: &PolicyContext<'_>) -> Vec<Operation> {
        vec![Operation::Add]
    }
}

/// Reject-everything policy, useful for tests that need to assert the
/// controller honors `[]` as "drop the event".
#[derive(Clone, Copy, Debug, Default)]
pub struct RefusePolicy;

#[async_trait]
impl Policy for RefusePolicy {
    async fn decide(&self, _ctx: &PolicyContext<'_>) -> Vec<Operation> {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use serde_json::json;

    use ditto_core::{EventId, ScopeId, Slot};

    use super::*;

    fn ev() -> Event {
        let payload = json!({"content": "x"});
        Event {
            event_id: EventId::from_payload(&payload).unwrap(),
            prev_event_id: None,
            tenant_id: TenantId::new(),
            scope_id: ScopeId::new(),
            source_id: "s".into(),
            slot: Slot::EpisodicIndex,
            payload,
            timestamp: Utc::now(),
        }
    }

    #[tokio::test]
    async fn heuristic_policy_emits_single_add() {
        let event = ev();
        let ctx = PolicyContext {
            tenant_id: event.tenant_id,
            event: &event,
            recent: &[],
        };
        let ops = HeuristicPolicy.decide(&ctx).await;
        assert_eq!(ops.len(), 1);
        assert!(matches!(ops[0], Operation::Add));
    }

    #[tokio::test]
    async fn refuse_policy_emits_empty_sequence() {
        let event = ev();
        let ctx = PolicyContext {
            tenant_id: event.tenant_id,
            event: &event,
            recent: &[],
        };
        let ops = RefusePolicy.decide(&ctx).await;
        assert!(ops.is_empty());
    }
}
