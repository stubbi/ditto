use crate::capabilities::CapabilitySet;
use crate::Error;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

/// Stable identifier for a tool. Matches the MCP server's tool name when the
/// tool is exposed via MCP — internal tools use any prefix-stable string.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct ToolId(pub String);

impl ToolId {
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }
}

/// SHA-256 of the canonical-JSON schema. Identical schemas across tools
/// collide on this hash and are deduped in `Projected` — caching across turns
/// becomes trivial.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct SchemaHash(pub [u8; 32]);

impl SchemaHash {
    pub fn hex(&self) -> String {
        let mut out = String::with_capacity(64);
        for b in self.0 {
            use std::fmt::Write;
            let _ = write!(&mut out, "{b:02x}");
        }
        out
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolKind {
    /// In-process Rust tool.
    Builtin,
    /// MCP server tool.
    Mcp,
    /// Provider-executed server-side tool (Anthropic web search, OpenAI
    /// code-interpreter). The model invokes it; the provider runs it.
    ProviderNative,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Tool {
    pub id: ToolId,
    pub kind: ToolKind,
    pub description: String,
    /// JSON schema for the tool's input. Stored as `serde_json::Value` so that
    /// `to_canonical_bytes` produces a stable byte string for hashing.
    pub input_schema: serde_json::Value,
    /// Per-channel/role static filter; tools missing from a channel's allow
    /// list never reach projection.
    pub channels: Vec<String>,
}

impl Tool {
    pub fn schema_hash(&self) -> Result<SchemaHash, Error> {
        let bytes = ditto_core::canonical::to_canonical_bytes(&self.input_schema)?;
        let digest = Sha256::digest(&bytes);
        let mut out = [0u8; 32];
        out.copy_from_slice(&digest);
        Ok(SchemaHash(out))
    }
}

/// Per-turn projection request — the agent loop hands one of these to
/// `ToolRegistry::project` at every step.
pub struct TurnProjection<'a> {
    pub channel: Option<&'a str>,
    pub allowed_kinds: &'a [ToolKind],
    pub budget_tokens: usize,
    pub mode: ProjectionMode,
    pub provider_caps: &'a dyn CapabilitySet,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ProjectionMode {
    /// Classic JSON-schema-inline. Used when (a) tool count × schema size
    /// stays under budget, and (b) the provider doesn't support Search/Code.
    Inline,
    /// Single `tools.search()` shim; client fetches schemas JIT when the
    /// model invokes search. Mirrors Claude Code v2.1.7's pattern.
    Search { index_tool: ToolId },
    /// MCP-as-code-module: provider receives an import-style shim and the
    /// agent writes `gdrive.get_transcript(...)` instead of receiving the
    /// schema inline. Mirrors Anthropic's Nov 2025 pattern; 98.7% reduction.
    CodeExecution { entrypoint_module: String },
}

/// Result of projecting a registry against a turn. Holds deduped schemas in a
/// `BTreeMap` so the wire serialization is deterministic.
#[derive(Clone, Debug, Default)]
pub struct Projected {
    pub mode: Option<ProjectionMode>,
    pub tools: Vec<ToolId>,
    pub schemas: BTreeMap<SchemaHash, Arc<Vec<u8>>>,
}

#[derive(Default)]
pub struct ToolRegistry {
    by_id: HashMap<ToolId, Arc<Tool>>,
    schema_cache: HashMap<SchemaHash, Arc<Vec<u8>>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, tool: Tool) -> Result<SchemaHash, Error> {
        let hash = tool.schema_hash()?;
        if let std::collections::hash_map::Entry::Vacant(e) = self.schema_cache.entry(hash) {
            let bytes = ditto_core::canonical::to_canonical_bytes(&tool.input_schema)?;
            e.insert(Arc::new(bytes));
        }
        self.by_id.insert(tool.id.clone(), Arc::new(tool));
        Ok(hash)
    }

    pub fn len(&self) -> usize {
        self.by_id.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_id.is_empty()
    }

    pub fn get(&self, id: &ToolId) -> Option<&Arc<Tool>> {
        self.by_id.get(id)
    }

    /// Project the registry against a turn.
    ///
    /// v0 logic: filter by channel + allowed kind, dedupe schemas by hash. The
    /// mode-selection heuristic (Inline / Search / CodeExecution) lives in
    /// `select_mode` so it's testable in isolation; callers can also force a
    /// mode through `TurnProjection::mode`.
    pub fn project(&self, turn: TurnProjection<'_>) -> Projected {
        let mut tools = Vec::new();
        let mut schemas: BTreeMap<SchemaHash, Arc<Vec<u8>>> = BTreeMap::new();

        let mut candidates: Vec<&Arc<Tool>> = self
            .by_id
            .values()
            .filter(|t| turn.allowed_kinds.contains(&t.kind))
            .filter(|t| match turn.channel {
                None => true,
                Some(ch) => t.channels.is_empty() || t.channels.iter().any(|c| c == ch),
            })
            .collect();
        candidates.sort_by(|a, b| a.id.cmp(&b.id));

        for tool in candidates {
            let Ok(hash) = tool.schema_hash() else { continue };
            tools.push(tool.id.clone());
            if let Some(bytes) = self.schema_cache.get(&hash) {
                schemas.entry(hash).or_insert_with(|| bytes.clone());
            }
        }

        Projected {
            mode: Some(turn.mode),
            tools,
            schemas,
        }
    }
}
