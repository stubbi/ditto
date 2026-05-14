//! The Procedural slot: skill packages with active/deprecated/archived
//! lifecycle.
//!
//! v0 ships the metadata index. Skill *execution* is a separate concern that
//! lives in `ditto-skills` (forthcoming) — this module only tracks "what
//! skills exist, who owns them, when were they last used, do their tests
//! pass". That metadata is what the dream-cycle metabolism rules consult.
//!
//! Architecture deviation note: the v2 spec lists `skill_id` as a global
//! PRIMARY KEY in the procedural table. We use composite `(tenant_id,
//! skill_id)` instead — globally-unique skill IDs would require every
//! deployment to coordinate on a namespace, and there is no operational
//! benefit to forbidding two tenants from each having a "deploy" skill.
//! The architecture doc will be updated to match in a follow-up.

use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::error::Error;
use crate::id::{ScopeId, TenantId};

/// Stable identifier for a skill within a tenant. Slash-delimited paths are
/// fine ("auth/login-helper"); the storage layer treats it as opaque.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct SkillId(pub String);

impl SkillId {
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for SkillId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl FromStr for SkillId {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.is_empty() {
            return Err(Error::Other("skill_id must be non-empty".into()));
        }
        Ok(Self::new(s))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillStatus {
    /// Available for invocation.
    Active,
    /// Metabolism rule (last_used > 30d or tests_pass < 0.7) marked it for
    /// removal. Still queryable but the controller will not surface it for
    /// new use.
    Deprecated,
    /// Removed from consideration entirely. Kept for audit; never invoked.
    Archived,
}

impl SkillStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Deprecated => "deprecated",
            Self::Archived => "archived",
        }
    }
}

impl FromStr for SkillStatus {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "active" => Self::Active,
            "deprecated" => Self::Deprecated,
            "archived" => Self::Archived,
            other => return Err(Error::Other(format!("unknown skill status: {other}"))),
        })
    }
}

/// A skill as persisted in the procedural index.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Skill {
    pub skill_id: SkillId,
    pub tenant_id: TenantId,
    pub scope_id: ScopeId,
    pub version: String,
    /// Filesystem path to the skill package (relative to a per-tenant
    /// skills root, or absolute — the metadata index treats it as a string).
    pub path: String,
    pub last_used: Option<DateTime<Utc>>,
    /// Pass-rate over the most recent test run [0.0, 1.0]. `None` until the
    /// skill has been tested.
    pub tests_pass: Option<f32>,
    pub status: SkillStatus,
}

/// New-skill input — what `register_skill` accepts.
#[derive(Clone, Debug)]
pub struct NewSkill {
    pub skill_id: SkillId,
    pub tenant_id: TenantId,
    pub scope_id: ScopeId,
    pub version: String,
    pub path: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skill_id_round_trips_through_str() {
        let id = SkillId::from_str("auth/login").unwrap();
        assert_eq!(id.as_str(), "auth/login");
        assert_eq!(id.to_string(), "auth/login");
    }

    #[test]
    fn empty_skill_id_rejected() {
        assert!(SkillId::from_str("").is_err());
    }

    #[test]
    fn status_round_trips_through_str() {
        for s in [SkillStatus::Active, SkillStatus::Deprecated, SkillStatus::Archived] {
            assert_eq!(SkillStatus::from_str(s.as_str()).unwrap(), s);
        }
        assert!(SkillStatus::from_str("garbage").is_err());
    }
}
