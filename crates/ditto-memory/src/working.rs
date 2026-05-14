//! The Working slot: in-context structured block, end-of-turn lifetime.
//!
//! Working memory is not persisted to storage. It lives in the agent process
//! for the duration of a turn and is rendered deterministically into the
//! system prompt. Biological analogue: Baddeley focus / Cowan focus — a
//! small bounded working set the agent is actively manipulating, distinct
//! from the long-term episodic / NC-graph state.
//!
//! Deterministic render is the contract: given the same `WorkingMemory`
//! state, `render_markdown()` produces byte-identical output. That's what
//! lets prompt caching across turns actually hit — Anthropic's cache_control
//! is bytewise, not semantic.
//!
//! Bounded by `capacity`: when the observation ring exceeds `capacity` the
//! oldest entry is dropped. The agent loop is expected to commit
//! soon-to-be-dropped observations into episodic memory before they fall off
//! the ring (the controller's write_path handles that).

use std::collections::VecDeque;
use std::fmt::Write;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObservationKind {
    /// User-supplied input — verbatim text the user wrote.
    UserInput,
    /// Tool result returned to the agent — verbatim payload.
    ToolResult,
    /// Agent-emitted intermediate reasoning or plan step.
    AgentThought,
    /// Anything else — system event, scheduled trigger, etc.
    Other,
}

impl ObservationKind {
    fn render(&self) -> &'static str {
        match self {
            Self::UserInput => "user",
            Self::ToolResult => "tool",
            Self::AgentThought => "agent",
            Self::Other => "other",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Observation {
    pub at: DateTime<Utc>,
    pub kind: ObservationKind,
    pub text: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WorkingMemory {
    capacity: usize,
    goal: Option<String>,
    sub_goal: Option<String>,
    hypothesis: Option<String>,
    observations: VecDeque<Observation>,
}

impl WorkingMemory {
    /// New working memory with a bounded observation ring of size `capacity`.
    pub fn new(capacity: usize) -> Self {
        assert!(capacity > 0, "WorkingMemory capacity must be positive");
        Self {
            capacity,
            goal: None,
            sub_goal: None,
            hypothesis: None,
            observations: VecDeque::with_capacity(capacity),
        }
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    pub fn goal(&self) -> Option<&str> {
        self.goal.as_deref()
    }
    pub fn sub_goal(&self) -> Option<&str> {
        self.sub_goal.as_deref()
    }
    pub fn hypothesis(&self) -> Option<&str> {
        self.hypothesis.as_deref()
    }
    pub fn observations(&self) -> impl Iterator<Item = &Observation> {
        self.observations.iter()
    }

    pub fn set_goal(&mut self, goal: impl Into<String>) {
        self.goal = Some(goal.into());
    }
    pub fn clear_goal(&mut self) {
        self.goal = None;
    }
    pub fn set_sub_goal(&mut self, sub: impl Into<String>) {
        self.sub_goal = Some(sub.into());
    }
    pub fn clear_sub_goal(&mut self) {
        self.sub_goal = None;
    }
    pub fn set_hypothesis(&mut self, h: impl Into<String>) {
        self.hypothesis = Some(h.into());
    }
    pub fn clear_hypothesis(&mut self) {
        self.hypothesis = None;
    }

    /// Push an observation, evicting the oldest if the ring is full.
    pub fn observe(
        &mut self,
        kind: ObservationKind,
        text: impl Into<String>,
        at: DateTime<Utc>,
    ) -> Option<Observation> {
        let evicted = if self.observations.len() >= self.capacity {
            self.observations.pop_front()
        } else {
            None
        };
        self.observations.push_back(Observation {
            at,
            kind,
            text: text.into(),
        });
        evicted
    }

    /// End-of-turn: drop every observation, leave goal / sub_goal / hypothesis
    /// in place. The agent loop calls this between turns.
    pub fn clear_observations(&mut self) {
        self.observations.clear();
    }

    /// Full reset — also clears goal / sub_goal / hypothesis. Used when the
    /// agent's task changes entirely (new conversation).
    pub fn clear(&mut self) {
        self.goal = None;
        self.sub_goal = None;
        self.hypothesis = None;
        self.observations.clear();
    }

    /// Deterministic Markdown render. Two calls against the same state must
    /// produce byte-identical output — that's what makes prompt-cache hits
    /// possible across turns. Observations are rendered most-recent-first so
    /// truncation at the bottom of the block (when context is tight) drops
    /// the oldest content first.
    pub fn render_markdown(&self) -> String {
        let mut out = String::with_capacity(256 + self.observations.len() * 80);
        out.push_str("# Working Memory\n\n");

        out.push_str("## Goal\n");
        out.push_str(self.goal.as_deref().unwrap_or("(none)"));
        out.push_str("\n\n");

        out.push_str("## Sub-goal\n");
        out.push_str(self.sub_goal.as_deref().unwrap_or("(none)"));
        out.push_str("\n\n");

        out.push_str("## Current hypothesis\n");
        out.push_str(self.hypothesis.as_deref().unwrap_or("(none)"));
        out.push_str("\n\n");

        out.push_str("## Recent observations (most recent first)\n");
        if self.observations.is_empty() {
            out.push_str("(none)\n");
        } else {
            for ob in self.observations.iter().rev() {
                let _ = writeln!(
                    out,
                    "- [{}, {}] {}",
                    ob.kind.render(),
                    ob.at.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
                    ob.text,
                );
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(year: i32, month: u32, day: u32, h: u32, m: u32, s: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(year, month, day, h, m, s).unwrap()
    }

    #[test]
    fn new_renders_placeholders() {
        let wm = WorkingMemory::new(5);
        let r = wm.render_markdown();
        assert!(r.contains("## Goal\n(none)"));
        assert!(r.contains("## Sub-goal\n(none)"));
        assert!(r.contains("## Current hypothesis\n(none)"));
        assert!(r.contains("## Recent observations (most recent first)\n(none)"));
    }

    #[test]
    fn setting_goal_updates_render() {
        let mut wm = WorkingMemory::new(5);
        wm.set_goal("Refactor the auth layer");
        let r = wm.render_markdown();
        assert!(r.contains("## Goal\nRefactor the auth layer\n"));
    }

    #[test]
    fn observations_render_most_recent_first() {
        let mut wm = WorkingMemory::new(5);
        wm.observe(
            ObservationKind::UserInput,
            "alpha",
            at(2026, 5, 14, 12, 0, 0),
        );
        wm.observe(
            ObservationKind::ToolResult,
            "beta",
            at(2026, 5, 14, 12, 0, 1),
        );
        let r = wm.render_markdown();
        let alpha_pos = r.find("alpha").unwrap();
        let beta_pos = r.find("beta").unwrap();
        assert!(
            beta_pos < alpha_pos,
            "most recent (beta) must render before older (alpha)"
        );
    }

    #[test]
    fn observation_ring_evicts_oldest_when_full() {
        let mut wm = WorkingMemory::new(2);
        let evicted_a = wm.observe(ObservationKind::Other, "a", at(2026, 5, 14, 12, 0, 0));
        let evicted_b = wm.observe(ObservationKind::Other, "b", at(2026, 5, 14, 12, 0, 1));
        let evicted_c = wm.observe(ObservationKind::Other, "c", at(2026, 5, 14, 12, 0, 2));
        assert!(evicted_a.is_none());
        assert!(evicted_b.is_none());
        let e = evicted_c.expect("third push must evict the first");
        assert_eq!(e.text, "a");
        let r = wm.render_markdown();
        assert!(!r.contains("a]"));
        assert!(r.contains("b"));
        assert!(r.contains("c"));
    }

    #[test]
    fn render_is_byte_deterministic() {
        let mut wm = WorkingMemory::new(3);
        wm.set_goal("g");
        wm.set_hypothesis("h");
        wm.observe(ObservationKind::AgentThought, "x", at(2026, 5, 14, 12, 0, 0));
        let a = wm.render_markdown();
        let b = wm.render_markdown();
        assert_eq!(a, b, "render must be byte-identical for prompt caching");
    }

    #[test]
    fn clear_observations_preserves_goal_and_hypothesis() {
        let mut wm = WorkingMemory::new(5);
        wm.set_goal("Investigate flaky test");
        wm.set_hypothesis("Race in fixture setup");
        wm.observe(ObservationKind::Other, "obs", at(2026, 5, 14, 12, 0, 0));
        wm.clear_observations();
        assert_eq!(wm.goal(), Some("Investigate flaky test"));
        assert_eq!(wm.hypothesis(), Some("Race in fixture setup"));
        assert_eq!(wm.observations().count(), 0);
    }

    #[test]
    fn full_clear_drops_everything() {
        let mut wm = WorkingMemory::new(5);
        wm.set_goal("g");
        wm.set_sub_goal("s");
        wm.set_hypothesis("h");
        wm.observe(ObservationKind::Other, "x", at(2026, 5, 14, 12, 0, 0));
        wm.clear();
        assert!(wm.goal().is_none());
        assert!(wm.sub_goal().is_none());
        assert!(wm.hypothesis().is_none());
        assert_eq!(wm.observations().count(), 0);
    }

    #[test]
    fn timestamps_render_as_rfc3339_z() {
        let mut wm = WorkingMemory::new(2);
        wm.observe(
            ObservationKind::UserInput,
            "hi",
            at(2026, 5, 14, 12, 0, 0),
        );
        let r = wm.render_markdown();
        assert!(r.contains("2026-05-14T12:00:00Z"));
    }
}
