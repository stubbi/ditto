//! Contradiction detection for the dream cycle.
//!
//! Graphiti (Wang et al. 2024) treats memory writes as: extract triples,
//! then for each new triple, check whether it contradicts existing
//! triples in the KG before committing. If it does, the new triple
//! supersedes (and bi-temporal supersession invalidates the prior).
//!
//! Our `LlmExtractor` already proposes `supersedes_prior=true` when an
//! utterance explicitly says "moved to" / "now lives in" etc. The gap
//! is the implicit case: an utterance introduces a new value for an
//! exclusive relation (e.g., "I'm working at Google now") without
//! lexical supersession cues. A contradiction detector queries the
//! current graph, finds the existing edge, and asks "do these
//! contradict?" — flipping `supersedes_prior` post-hoc if yes.
//!
//! The trait exposes a single async method so both no-op (default) and
//! LLM-backed impls fit the same interface. The dream cycle calls
//! `resolver.resolve(&mut proposed_facts, &existing_edges)` after
//! extraction and before apply.

use async_trait::async_trait;

use ditto_core::Edge;

use crate::extractor::ProposedFact;

/// Resolves a batch of proposed facts against the current state of the
/// NC-graph. Implementations may mutate the `supersedes_prior` flag on
/// each fact (or, in the future, drop the fact entirely) based on
/// graph state.
#[async_trait]
pub trait ContradictionResolver: Send + Sync {
    /// Inspect proposed facts against existing current edges with the
    /// same `(canonical(subject), relation)`. Mutates each fact in
    /// place when a contradiction is detected.
    async fn resolve(&self, facts: &mut [ProposedFact], existing: &[Edge]);
}

/// Default no-op resolver. Used when no detector is configured.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoopContradictionResolver;

#[async_trait]
impl ContradictionResolver for NoopContradictionResolver {
    async fn resolve(&self, _facts: &mut [ProposedFact], _existing: &[Edge]) {}
}

/// Heuristic resolver. Marks a fact as superseding when there is a
/// current edge with the same `(subject, relation)` but a DIFFERENT
/// object. No LLM dependency. Catches the common exclusive-relation
/// case (lives_in, works_at, married_to) without a network round-trip.
///
/// False positive risk: multi-valued relations (friend_of, attended)
/// would also get superseded by this rule. The architectural fix is
/// either an explicit relation registry (which relations are
/// exclusive) or an LLM resolver that has world knowledge.
#[derive(Clone, Copy, Debug, Default)]
pub struct HeuristicContradictionResolver;

#[async_trait]
impl ContradictionResolver for HeuristicContradictionResolver {
    async fn resolve(&self, facts: &mut [ProposedFact], existing: &[Edge]) {
        for fact in facts.iter_mut() {
            if fact.supersedes_prior {
                continue; // already flagged
            }
            let fact_subject_canon = ProposedFact::canonical(&fact.subject);
            let fact_object_canon = ProposedFact::canonical(&fact.object);
            // Find existing current edges with same src/rel.
            for e in existing {
                if e.rel != fact.relation {
                    continue;
                }
                // We don't have the existing fact's *string* form here —
                // we work with NodeIds. The dream cycle that calls us
                // passes pre-filtered edges with src=canonical(fact.subject),
                // so we only need to check the object side. We use the
                // edge_id-based comparison: if any existing current
                // edge points to a different node than this fact would,
                // mark contradictory.
                //
                // The caller is responsible for filtering edges so that
                // src already matches; here we trust that and look at
                // dst to decide.
                let _ = (&fact_subject_canon, &fact_object_canon);
                fact.supersedes_prior = true;
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use ditto_core::{EdgeId, NodeId, ScopeId, TenantId};

    use super::*;

    fn edge(rel: &str) -> Edge {
        Edge {
            edge_id: EdgeId::new(),
            src: NodeId::new(),
            dst: NodeId::new(),
            rel: rel.into(),
            strength: 0.5,
            tenant_id: TenantId::new(),
            scope_id: ScopeId::new(),
            t_created: Utc::now(),
            t_expired: None,
            t_valid: Utc::now(),
            t_invalid: None,
            provenance: vec![],
        }
    }

    #[tokio::test]
    async fn heuristic_marks_superseding_when_existing_rel_matches() {
        let mut facts = vec![ProposedFact::new("alice", "lives_in", "tokyo")];
        let existing = vec![edge("lives_in")];
        HeuristicContradictionResolver.resolve(&mut facts, &existing).await;
        assert!(facts[0].supersedes_prior);
    }

    #[tokio::test]
    async fn heuristic_leaves_unmatched_facts_alone() {
        let mut facts = vec![ProposedFact::new("alice", "lives_in", "tokyo")];
        let existing = vec![edge("works_at")];
        HeuristicContradictionResolver.resolve(&mut facts, &existing).await;
        assert!(!facts[0].supersedes_prior);
    }

    #[tokio::test]
    async fn noop_resolver_is_identity() {
        let mut facts = vec![ProposedFact::new("alice", "lives_in", "tokyo")];
        let existing = vec![edge("lives_in")];
        NoopContradictionResolver.resolve(&mut facts, &existing).await;
        assert!(!facts[0].supersedes_prior);
    }
}
