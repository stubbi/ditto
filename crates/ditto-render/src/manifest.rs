//! Render manifest: `.ditto-render.json` at the output root.
//!
//! Records what was rendered, when, and the content hash per page. Lets
//! re-renders detect no-op (no graph changes → no page churn) and lets
//! external tools verify integrity offline.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use ditto_core::{ScopeId, TenantId};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Manifest {
    pub version: u32,
    pub regenerated_at: DateTime<Utc>,
    pub tenant_id: TenantId,
    pub scope_id: Option<ScopeId>,
    pub pages: BTreeMap<String, String>,
    pub index_hash: String,
}

impl Manifest {
    pub const PATH: &'static str = ".ditto-render.json";

    pub fn page_count(&self) -> usize {
        self.pages.len()
    }
}
