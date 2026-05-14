//! Pluggable fact extractor for the dream cycle.
//!
//! An `Extractor` turns an episodic payload into a list of `ProposedFact`s
//! the consolidator considers for NC-graph commit. v0 ships two impls:
//!
//! - `RuleExtractor` — deterministic regex/heuristic patterns. No LLM
//!   dependency, no network calls, byte-deterministic across runs.
//!   Handles a focused set of patterns ("X lives in Y", "X moved to Y",
//!   "X is allergic to Y", "X works at Z") that exercise the
//!   contradiction-and-supersession path the architecture commits to.
//! - `NoopExtractor` — returns no facts. Used by deployments that prefer
//!   to keep the NC-graph empty until an LLM extractor lands.
//!
//! The architecture commits to an LLM-driven Observer/Reflector pipeline
//! for "real" extraction. That impl will live in `ditto-extractors` (or
//! similar) and route through `ditto-models` so it composes with the
//! provider/auth layer. The trait surface here is what it'll implement.

use async_trait::async_trait;
use ditto_core::{Event, NodeId, ScopeId, SupersedePolicy, TenantId};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A fact the extractor proposes the dream cycle commit to NC-graph.
///
/// Subject and object are *human-readable strings* — the consolidator
/// resolves them to NodeIds via canonicalization (lowercased, trimmed,
/// hashed). Different extractors all see the same "user" string and end
/// up writing to the same node.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProposedFact {
    pub subject: String,
    pub relation: String,
    pub object: String,
    /// Strength in [0.0, 1.0]; how confident the extractor is. Storage
    /// uses this directly as `Edge.strength`.
    pub confidence: f32,
    /// Whether this fact contradicts prior values of the same
    /// `(subject, relation)` pair. When true, the dream cycle commits
    /// with `SameSrcRelDst` supersession (architecturally a typo — should
    /// be `AnyWithSameRelation` for the "exclusive" lives_in case).
    /// `RuleExtractor` sets this true for verbs like "moved" / "now"
    /// / "switched" that imply replacement of a prior value.
    pub supersedes_prior: bool,
}

impl ProposedFact {
    pub fn new(
        subject: impl Into<String>,
        relation: impl Into<String>,
        object: impl Into<String>,
    ) -> Self {
        Self {
            subject: subject.into(),
            relation: relation.into(),
            object: object.into(),
            confidence: 0.8,
            supersedes_prior: false,
        }
    }

    pub fn supersedes(mut self) -> Self {
        self.supersedes_prior = true;
        self
    }

    pub fn with_confidence(mut self, c: f32) -> Self {
        self.confidence = c.clamp(0.0, 1.0);
        self
    }

    /// Canonical node-id key for a string. Lowercased and trimmed so
    /// "User" / "user" / " user " collapse to the same node.
    pub fn canonical(s: &str) -> String {
        s.trim().to_lowercase()
    }

    /// Supersession policy this fact wants. Architecture rule of thumb:
    /// `lives_in`, `works_at`, `married_to` are exclusive (one current
    /// value) → `AnyWithSameRelation`. Multi-valued relations like
    /// `friend_of` or `attended` would use `SameSrcRelDst` instead, but
    /// `RuleExtractor` doesn't propose those yet.
    pub fn supersede_policy(&self) -> Option<SupersedePolicy> {
        if self.supersedes_prior {
            Some(SupersedePolicy::AnyWithSameRelation)
        } else {
            None
        }
    }
}

/// What an `Extractor` returns: candidates and confidence to short-circuit
/// when nothing is recognized.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Extraction {
    pub facts: Vec<ProposedFact>,
}

impl Extraction {
    pub fn empty() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.facts.is_empty()
    }
}

#[async_trait]
pub trait Extractor: Send + Sync {
    /// Extract facts from an episodic event. The default `RuleExtractor`
    /// is synchronous in practice, but the trait is async so LLM-based
    /// extractors fit the same interface.
    async fn extract(&self, event: &Event) -> Extraction;
}

/// No-op extractor. The dream cycle skips NC-graph writes entirely when
/// this is configured. Useful for early deployments before extraction
/// quality is trustworthy.
pub struct NoopExtractor;

#[async_trait]
impl Extractor for NoopExtractor {
    async fn extract(&self, _event: &Event) -> Extraction {
        Extraction::empty()
    }
}

/// Deterministic, regex-free pattern extractor.
///
/// Implementation note: we deliberately avoid the `regex` crate so the
/// scoping stays minimal. Pattern matching here is hand-rolled
/// "split-and-search". Each pattern is documented inline.
pub struct RuleExtractor;

impl Default for RuleExtractor {
    fn default() -> Self {
        Self
    }
}

impl RuleExtractor {
    pub fn new() -> Self {
        Self
    }

    fn content_of(event: &Event) -> Option<String> {
        match &event.payload {
            Value::Object(map) => map
                .get("content")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            Value::String(s) => Some(s.clone()),
            _ => None,
        }
    }

    /// Pattern: `"<subject> moved to <object>"` (any subject — "User",
    /// "Alice", "I"). Variants: "moved from X to Y" / "now lives in Y" /
    /// "switched cities to Y". Each yields a supersession-flagged
    /// `lives_in` fact. The "from X" half is ignored (the prior value is
    /// already in the graph; supersession takes care of invalidation).
    fn extract_move(content: &str) -> Option<ProposedFact> {
        let lower = content.to_lowercase();
        let subject = "user"; // canonicalized; matches `extract_lives_in`'s subject
        for trigger in ["moved to ", "now lives in ", "relocated to "] {
            if let Some(idx) = lower.find(trigger) {
                let after = &lower[idx + trigger.len()..];
                let obj = first_phrase(after);
                if !obj.is_empty() {
                    return Some(
                        ProposedFact::new(subject, "lives_in", obj.trim_end_matches('.'))
                            .supersedes(),
                    );
                }
            }
        }
        None
    }

    /// Pattern: `"<subject> lives in <object>"` — stable state, NOT a
    /// supersession. If the user has never moved, this is just the
    /// current residence. The dream cycle's contradiction check will
    /// upgrade it to a supersede if a later "moved" event lands.
    fn extract_lives_in(content: &str) -> Option<ProposedFact> {
        let lower = content.to_lowercase();
        for trigger in ["lives in ", "is from ", "based in "] {
            if let Some(idx) = lower.find(trigger) {
                let after = &lower[idx + trigger.len()..];
                let obj = first_phrase(after);
                if !obj.is_empty() {
                    return Some(ProposedFact::new(
                        "user",
                        "lives_in",
                        obj.trim_end_matches('.'),
                    ));
                }
            }
        }
        None
    }

    /// Pattern: `"<subject> works at <object>"` — same shape as lives_in.
    /// "switched to <object>" / "joined <object>" upgrade to supersedes.
    fn extract_works_at(content: &str) -> Option<ProposedFact> {
        let lower = content.to_lowercase();
        for trigger in ["works at ", "employed by "] {
            if let Some(idx) = lower.find(trigger) {
                let obj = first_phrase(&lower[idx + trigger.len()..]);
                if !obj.is_empty() {
                    return Some(ProposedFact::new(
                        "user",
                        "works_at",
                        obj.trim_end_matches('.').trim_end_matches(','),
                    ));
                }
            }
        }
        for trigger in ["joined ", "switched to "] {
            if let Some(idx) = lower.find(trigger) {
                let obj = first_phrase(&lower[idx + trigger.len()..]);
                if !obj.is_empty() {
                    return Some(
                        ProposedFact::new("user", "works_at", obj.trim_end_matches('.'))
                            .supersedes(),
                    );
                }
            }
        }
        None
    }

    /// Pattern: `"<subject> is allergic to <object>"`. Allergies aren't
    /// inherently exclusive (one can have multiple), so this never
    /// supersedes — each new allergy is additive.
    fn extract_allergy(content: &str) -> Option<ProposedFact> {
        let lower = content.to_lowercase();
        for trigger in ["allergic to "] {
            if let Some(idx) = lower.find(trigger) {
                let obj = first_phrase(&lower[idx + trigger.len()..]);
                if !obj.is_empty() {
                    return Some(ProposedFact::new(
                        "user",
                        "allergic_to",
                        obj.trim_end_matches('.'),
                    ));
                }
            }
        }
        None
    }
}

#[async_trait]
impl Extractor for RuleExtractor {
    async fn extract(&self, event: &Event) -> Extraction {
        let Some(content) = Self::content_of(event) else {
            return Extraction::empty();
        };
        let mut facts = Vec::new();
        // Movement patterns take precedence over plain lives_in so that
        // "moved to Berlin" doesn't also get extracted as a literal
        // residence (which would skip supersession).
        if let Some(f) = Self::extract_move(&content) {
            facts.push(f);
        } else if let Some(f) = Self::extract_lives_in(&content) {
            facts.push(f);
        }
        if let Some(f) = Self::extract_works_at(&content) {
            facts.push(f);
        }
        if let Some(f) = Self::extract_allergy(&content) {
            facts.push(f);
        }
        Extraction { facts }
    }
}

/// First "phrase" of a string: characters up to the first comma, period,
/// or " and ". Lowercase input; trims surrounding whitespace.
fn first_phrase(s: &str) -> String {
    let mut end = s.len();
    for (i, c) in s.char_indices() {
        if c == ',' || c == '.' {
            end = i;
            break;
        }
    }
    // Trim " and X..." tail.
    let candidate = &s[..end];
    if let Some(idx) = candidate.find(" and ") {
        return candidate[..idx].trim().to_string();
    }
    candidate.trim().to_string()
}

/// Helper used by the dream-cycle code: turns a `ProposedFact`'s string
/// subject/object into a stable NodeId via UUID v5 on the canonicalized
/// name. This is what gives "user" and "User" the same node across
/// extractor runs.
pub fn name_to_node_id(_tenant: TenantId, _scope: ScopeId, name: &str) -> NodeId {
    // We re-use a fixed namespace UUID so the mapping is stable across
    // tenants. Per-tenant separation comes from the `tenant_id` field on
    // the Node row itself, not the NodeId bytes.
    const NS: uuid::Uuid = uuid::Uuid::from_bytes([
        0x4d, 0x69, 0x74, 0x4f, 0x52, 0x75, 0x6c, 0x65, 0x2d, 0x4e, 0x6f, 0x64, 0x65, 0x49, 0x64,
        0x21,
    ]);
    let canon = ProposedFact::canonical(name);
    NodeId(uuid::Uuid::new_v5(&NS, canon.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ditto_core::Slot;
    use serde_json::json;

    fn ev(content: &str) -> Event {
        Event::new(
            TenantId::new(),
            ScopeId::new(),
            "test",
            Slot::EpisodicIndex,
            json!({"content": content}),
            chrono::Utc::now(),
            None,
        )
        .unwrap()
    }

    #[tokio::test]
    async fn moved_to_extracts_supersede_lives_in() {
        let e = ev("User moved to Berlin last week, starting at Beta Inc next Monday.");
        let out = RuleExtractor::new().extract(&e).await;
        let lives = out
            .facts
            .iter()
            .find(|f| f.relation == "lives_in")
            .unwrap();
        assert_eq!(lives.object, "berlin last week");
        assert!(lives.supersedes_prior);
        assert!(matches!(
            lives.supersede_policy(),
            Some(SupersedePolicy::AnyWithSameRelation)
        ));
    }

    #[tokio::test]
    async fn lives_in_extracts_non_supersede() {
        let e = ev("User lives in San Francisco, works at Acme Corp.");
        let out = RuleExtractor::new().extract(&e).await;
        let lives = out
            .facts
            .iter()
            .find(|f| f.relation == "lives_in")
            .unwrap();
        assert_eq!(lives.object, "san francisco");
        assert!(!lives.supersedes_prior);
    }

    #[tokio::test]
    async fn lives_in_and_works_at_both_extract() {
        let e = ev("User lives in San Francisco, works at Acme Corp.");
        let out = RuleExtractor::new().extract(&e).await;
        let rels: Vec<&str> = out.facts.iter().map(|f| f.relation.as_str()).collect();
        assert!(rels.contains(&"lives_in"));
        assert!(rels.contains(&"works_at"));
    }

    #[tokio::test]
    async fn allergic_to_extracts_additive_fact() {
        let e = ev("User is allergic to peanuts.");
        let out = RuleExtractor::new().extract(&e).await;
        let f = out
            .facts
            .iter()
            .find(|f| f.relation == "allergic_to")
            .unwrap();
        assert_eq!(f.object, "peanuts");
        assert!(!f.supersedes_prior); // multi-valued; never supersedes
    }

    #[tokio::test]
    async fn unrelated_content_extracts_nothing() {
        let e = ev("User asked about restaurants in Berlin near Mitte.");
        let out = RuleExtractor::new().extract(&e).await;
        // No move / lives_in / allergic / works_at trigger present.
        assert!(out.is_empty(), "got {:?}", out.facts);
    }

    #[tokio::test]
    async fn noop_extractor_returns_empty() {
        let e = ev("User lives in San Francisco.");
        let out = NoopExtractor.extract(&e).await;
        assert!(out.is_empty());
    }

    #[test]
    fn canonical_lowercases_and_trims() {
        assert_eq!(ProposedFact::canonical("  User  "), "user");
        assert_eq!(ProposedFact::canonical("BERLIN"), "berlin");
    }

    #[test]
    fn name_to_node_id_is_stable_across_runs() {
        let t = TenantId::new();
        let s = ScopeId::new();
        let a = name_to_node_id(t, s, "user");
        let b = name_to_node_id(t, s, "User");
        let c = name_to_node_id(t, s, " USER ");
        assert_eq!(a, b);
        assert_eq!(a, c);
    }
}
