//! Targeted Memory Reactivation cue.
//!
//! Biological precedent: Rasch et al. 2007. In Ditto, a cue biases the
//! next dream-cycle sweep to prioritize events whose content overlaps the
//! cue's focus tokens. Consumed by the cycle that processes it.

use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::Error;
use crate::id::{ScopeId, TenantId};

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct TmrCueId(pub Uuid);

impl TmrCueId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for TmrCueId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for TmrCueId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl FromStr for TmrCueId {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(Uuid::parse_str(s)?))
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TmrCue {
    pub cue_id: TmrCueId,
    pub tenant_id: TenantId,
    pub scope_id: Option<ScopeId>,
    pub focus: String,
    pub hint: Option<String>,
    pub set_at: DateTime<Utc>,
    pub consumed_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug)]
pub struct NewTmrCue {
    pub cue_id: TmrCueId,
    pub tenant_id: TenantId,
    pub scope_id: Option<ScopeId>,
    pub focus: String,
    pub hint: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cue_id_round_trip() {
        let a = TmrCueId::new();
        let b = TmrCueId::from_str(&a.to_string()).unwrap();
        assert_eq!(a, b);
    }
}
