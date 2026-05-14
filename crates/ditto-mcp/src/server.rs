//! MCP server impl.
//!
//! v0 tools exposed:
//!
//! - `write_event` — commit an episodic event; returns a signed Receipt.
//! - `search` — search the tenant's memory (BM25 / substring under the hood).
//! - `assert_node` — idempotent NC-graph node upsert.
//! - `write_fact` — insert an NC-graph edge with optional supersession.
//! - `current_edges_from` — current outgoing edges (point-in-time = now).
//! - `edges_from_at` — outgoing edges valid at a specific instant (time travel).
//! - `invalidate_edge` — set `t_invalid` on an edge.
//! - `verify_receipt` — verify a receipt against its event.
//!
//! All input args are typed Rust structs deriving `schemars::JsonSchema` so
//! the MCP client gets auto-generated input schemas. All return values are
//! JSON text content (the call-site can parse).

use std::str::FromStr;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use rmcp::{
    ErrorData as McpError, ServerHandler, ServiceExt,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{CallToolResult, Content, Implementation, ProtocolVersion, ServerCapabilities, ServerInfo},
    schemars,
    tool, tool_handler, tool_router,
    transport::stdio,
};
use serde::{Deserialize, Serialize};

use ditto_core::{
    Blob, BlobHash, EdgeId, NewEdge, NewNode, NodeId, Receipt, ScopeId, Slot, SupersedePolicy,
    TenantId,
};
#[cfg(test)]
use ditto_core::InstallKey;
use ditto_memory::{MemoryController, SearchMode, SearchQuery, Storage};

/// Run a Ditto MCP server on stdio. Blocks until the client disconnects.
pub async fn serve_stdio<S: Storage + 'static>(
    ctrl: Arc<MemoryController<S>>,
) -> anyhow::Result<()> {
    let server = DittoMcpServer::new(ctrl);
    let service = server.serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}

/// MCP server that wraps a [`MemoryController`] for any [`Storage`] backend.
#[derive(Clone)]
pub struct DittoMcpServer<S: Storage + 'static> {
    ctrl: Arc<MemoryController<S>>,
    tool_router: ToolRouter<Self>,
}

impl<S: Storage + 'static> DittoMcpServer<S> {
    pub fn new(ctrl: Arc<MemoryController<S>>) -> Self {
        Self {
            ctrl,
            tool_router: Self::tool_router(),
        }
    }
}

// --- Parameter structs ---

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct WriteEventParams {
    pub tenant: String,
    pub scope: String,
    pub source: String,
    /// One of: working | episodic_index | blob_store | nc_graph | nc_doc |
    /// procedural | reflective. Default `episodic_index`.
    #[serde(default)]
    pub slot: Option<String>,
    /// JSON payload. Identical payloads produce identical event_ids.
    pub payload: serde_json::Value,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SearchParams {
    pub tenant: String,
    pub query: String,
    #[serde(default)]
    pub scope: Option<String>,
    #[serde(default)]
    pub sources: Option<Vec<String>>,
    #[serde(default)]
    pub k: Option<usize>,
    /// "cheap" | "standard" | "deep". Default "standard".
    #[serde(default)]
    pub mode: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct AssertNodeParams {
    pub tenant: String,
    pub scope: String,
    pub node_type: String,
    pub properties: serde_json::Value,
    /// Optional pre-assigned node_id. If omitted, a fresh UUID is generated.
    #[serde(default)]
    pub node_id: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct WriteFactParams {
    pub tenant: String,
    pub scope: String,
    pub src: String,
    pub dst: String,
    pub rel: String,
    /// RFC 3339 timestamp; the fact starts being true at this instant.
    pub t_valid: String,
    #[serde(default)]
    pub t_invalid: Option<String>,
    #[serde(default)]
    pub strength: Option<f32>,
    /// "any_with_same_relation" or "same_src_rel_dst". Omit to skip
    /// supersession (multi-valued relations).
    #[serde(default)]
    pub supersede: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct EdgesFromParams {
    pub tenant: String,
    pub src: String,
    #[serde(default)]
    pub rel: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct EdgesFromAtParams {
    pub tenant: String,
    pub src: String,
    /// RFC 3339 instant.
    pub t: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct InvalidateEdgeParams {
    pub edge_id: String,
    pub t_invalid: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct PutBlobParams {
    pub tenant: String,
    /// Base64-encoded payload bytes.
    pub bytes_b64: String,
    /// Optional MIME hint. Defaults to application/octet-stream.
    #[serde(default)]
    pub content_type: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GetBlobParams {
    pub tenant: String,
    /// Lowercase hex SHA-256 of the blob payload (64 chars).
    pub hash: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct VerifyReceiptParams {
    /// A receipt JSON object (as returned by write_event).
    pub receipt: serde_json::Value,
}

// --- Tool handlers ---

#[tool_router]
impl<S: Storage + 'static> DittoMcpServer<S> {
    #[tool(description = "Commit an episodic event. Returns a signed Receipt. Idempotent on content-addressed event_id.")]
    async fn write_event(
        &self,
        params: Parameters<WriteEventParams>,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;
        let tenant = parse_tenant(&p.tenant)?;
        let scope = parse_scope(&p.scope)?;
        let slot = parse_slot(p.slot.as_deref().unwrap_or("episodic_index"))?;
        let receipt = self
            .ctrl
            .write(tenant, scope, p.source, slot, p.payload, Utc::now())
            .await
            .map_err(storage_err)?;
        ok_json(&receipt)
    }

    #[tool(description = "Search the tenant's memory. Returns ranked SearchResults with provenance.")]
    async fn search(
        &self,
        params: Parameters<SearchParams>,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;
        let mut q = SearchQuery::new(p.query, parse_tenant(&p.tenant)?);
        q.scope_id = p.scope.as_deref().map(parse_scope).transpose()?;
        q.sources = p.sources;
        if let Some(k) = p.k {
            q.k = k;
        }
        q.mode = parse_mode(p.mode.as_deref().unwrap_or("standard"))?;
        let results = self.ctrl.search(&q).await.map_err(storage_err)?;
        ok_json(&results)
    }

    #[tool(description = "Idempotent NC-graph node upsert. Returns the node (existing or newly created).")]
    async fn assert_node(
        &self,
        params: Parameters<AssertNodeParams>,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;
        let node_id = match p.node_id.as_deref() {
            Some(s) => NodeId::from_str(s).map_err(bad_arg)?,
            None => NodeId::new(),
        };
        let node = self
            .ctrl
            .assert_node(NewNode {
                node_id,
                tenant_id: parse_tenant(&p.tenant)?,
                scope_id: parse_scope(&p.scope)?,
                node_type: p.node_type,
                properties: p.properties,
                provenance: vec![],
            })
            .await
            .map_err(storage_err)?;
        ok_json(&node)
    }

    #[tool(description = "Insert an NC-graph edge with optional supersession of prior contradicting facts. Bi-temporal: caller supplies t_valid (when the fact starts being true).")]
    async fn write_fact(
        &self,
        params: Parameters<WriteFactParams>,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;
        let edge = NewEdge {
            edge_id: EdgeId::new(),
            src: NodeId::from_str(&p.src).map_err(bad_arg)?,
            dst: NodeId::from_str(&p.dst).map_err(bad_arg)?,
            rel: p.rel,
            strength: p.strength,
            tenant_id: parse_tenant(&p.tenant)?,
            scope_id: parse_scope(&p.scope)?,
            t_valid: parse_ts(&p.t_valid)?,
            t_invalid: p.t_invalid.as_deref().map(parse_ts).transpose()?,
            provenance: vec![],
            supersede: p.supersede.as_deref().map(parse_supersede).transpose()?,
        };
        let edge = self.ctrl.write_fact(edge).await.map_err(storage_err)?;
        ok_json(&edge)
    }

    #[tool(description = "Current outgoing edges from a node (t_expired IS NULL AND t_invalid IS NULL). Optional relation filter.")]
    async fn current_edges_from(
        &self,
        params: Parameters<EdgesFromParams>,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;
        let edges = self
            .ctrl
            .current_edges_from(
                parse_tenant(&p.tenant)?,
                NodeId::from_str(&p.src).map_err(bad_arg)?,
                p.rel.as_deref(),
            )
            .await
            .map_err(storage_err)?;
        ok_json(&edges)
    }

    #[tool(description = "Time-travel query: outgoing edges from a node that were valid at a specific instant.")]
    async fn edges_from_at(
        &self,
        params: Parameters<EdgesFromAtParams>,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;
        let edges = self
            .ctrl
            .edges_from_at(
                parse_tenant(&p.tenant)?,
                NodeId::from_str(&p.src).map_err(bad_arg)?,
                parse_ts(&p.t)?,
            )
            .await
            .map_err(storage_err)?;
        ok_json(&edges)
    }

    #[tool(description = "Mark an edge as no-longer-true at t_invalid. The edge stays queryable for historical reads.")]
    async fn invalidate_edge(
        &self,
        params: Parameters<InvalidateEdgeParams>,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;
        self.ctrl
            .invalidate_edge(
                EdgeId::from_str(&p.edge_id).map_err(bad_arg)?,
                parse_ts(&p.t_invalid)?,
            )
            .await
            .map_err(storage_err)?;
        ok_json(&serde_json::json!({"ok": true}))
    }

    #[tool(description = "Store a blob in the tenant's content-addressed store. Idempotent on SHA-256 of bytes. Returns {hash, bytelen, content_type}.")]
    async fn put_blob(
        &self,
        params: Parameters<PutBlobParams>,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;
        let tenant = parse_tenant(&p.tenant)?;
        let bytes = b64_decode(&p.bytes_b64)?;
        let blob = Blob::new(
            bytes,
            p.content_type
                .unwrap_or_else(|| "application/octet-stream".into()),
        );
        let bytelen = blob.bytes.len();
        let content_type = blob.content_type.clone();
        let hash = self
            .ctrl
            .put_blob(tenant, &blob)
            .await
            .map_err(storage_err)?;
        ok_json(&serde_json::json!({
            "hash": hash.to_hex(),
            "bytelen": bytelen,
            "content_type": content_type,
        }))
    }

    #[tool(description = "Fetch a blob by hash from the tenant's content-addressed store. Returns {hash, bytes_b64, content_type} or null when absent.")]
    async fn get_blob(
        &self,
        params: Parameters<GetBlobParams>,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;
        let tenant = parse_tenant(&p.tenant)?;
        let hash = BlobHash::from_hex(&p.hash).map_err(|e| bad_arg(format!("hash: {e}")))?;
        let blob = self
            .ctrl
            .get_blob(tenant, hash)
            .await
            .map_err(storage_err)?;
        match blob {
            Some(b) => ok_json(&serde_json::json!({
                "hash": hash.to_hex(),
                "content_type": b.content_type,
                "bytes_b64": b64_encode(&b.bytes),
            })),
            None => ok_json(&serde_json::Value::Null),
        }
    }

    #[tool(description = "Verify a receipt's signature against the original event. Returns {valid: bool}.")]
    async fn verify_receipt(
        &self,
        params: Parameters<VerifyReceiptParams>,
    ) -> Result<CallToolResult, McpError> {
        let receipt: Receipt =
            serde_json::from_value(params.0.receipt).map_err(|e| bad_arg(e.to_string()))?;
        let valid = self.ctrl.verify(&receipt).await.map_err(storage_err)?;
        ok_json(&serde_json::json!({"valid": valid}))
    }
}

#[tool_handler]
impl<S: Storage + 'static> ServerHandler for DittoMcpServer<S> {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            protocol_version: ProtocolVersion::default(),
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            server_info: Implementation {
                name: "ditto-mcp".into(),
                version: env!("CARGO_PKG_VERSION").into(),
                title: Some("Ditto Memory".into()),
                ..Default::default()
            },
            instructions: Some(
                "Ditto agent memory: bi-temporal NC-graph + signed episodic events + content-addressed blob store. \
                 Write facts with t_valid/t_invalid for time-travel queries; every write returns a SCITT-style signed Receipt.".into(),
            ),
        }
    }
}

// --- Helpers ---

fn parse_tenant(s: &str) -> Result<TenantId, McpError> {
    TenantId::from_str(s).map_err(|e| bad_arg(format!("tenant: {e}")))
}

fn parse_scope(s: &str) -> Result<ScopeId, McpError> {
    ScopeId::from_str(s).map_err(|e| bad_arg(format!("scope: {e}")))
}

fn parse_slot(s: &str) -> Result<Slot, McpError> {
    Ok(match s {
        "working" => Slot::Working,
        "episodic_index" | "episodic" => Slot::EpisodicIndex,
        "blob_store" | "blob" => Slot::BlobStore,
        "nc_graph" | "graph" => Slot::NcGraph,
        "nc_doc" | "doc" => Slot::NcDoc,
        "procedural" => Slot::Procedural,
        "reflective" => Slot::Reflective,
        other => return Err(bad_arg(format!("unknown slot: {other}"))),
    })
}

fn parse_mode(s: &str) -> Result<SearchMode, McpError> {
    Ok(match s {
        "cheap" => SearchMode::Cheap,
        "standard" => SearchMode::Standard,
        "deep" => SearchMode::Deep,
        other => return Err(bad_arg(format!("unknown mode: {other}"))),
    })
}

fn parse_supersede(s: &str) -> Result<SupersedePolicy, McpError> {
    Ok(match s {
        "any_with_same_relation" => SupersedePolicy::AnyWithSameRelation,
        "same_src_rel_dst" => SupersedePolicy::SameSrcRelDst,
        other => return Err(bad_arg(format!("unknown supersede policy: {other}"))),
    })
}

fn parse_ts(s: &str) -> Result<DateTime<Utc>, McpError> {
    DateTime::parse_from_rfc3339(s)
        .map(|t| t.with_timezone(&Utc))
        .map_err(|e| bad_arg(format!("timestamp: {e}")))
}

fn bad_arg<E: std::fmt::Display>(e: E) -> McpError {
    McpError::invalid_params(e.to_string(), None)
}

fn storage_err(e: ditto_memory::storage::StorageError) -> McpError {
    McpError::internal_error(e.to_string(), None)
}

fn ok_json<T: Serialize>(value: &T) -> Result<CallToolResult, McpError> {
    let text = serde_json::to_string_pretty(value)
        .map_err(|e| McpError::internal_error(format!("serialize: {e}"), None))?;
    Ok(CallToolResult::success(vec![Content::text(text)]))
}

const B64_ALPH: &[u8; 64] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn b64_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            if chunk.len() > 1 { chunk[1] } else { 0 },
            if chunk.len() > 2 { chunk[2] } else { 0 },
        ];
        out.push(B64_ALPH[(b[0] >> 2) as usize] as char);
        out.push(B64_ALPH[((b[0] & 0x03) << 4 | b[1] >> 4) as usize] as char);
        if chunk.len() > 1 {
            out.push(B64_ALPH[((b[1] & 0x0f) << 2 | b[2] >> 6) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(B64_ALPH[(b[2] & 0x3f) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

fn b64_decode(s: &str) -> Result<Vec<u8>, McpError> {
    let s = s.trim();
    if s.len() % 4 != 0 {
        return Err(bad_arg("base64 length must be a multiple of 4"));
    }
    let mut out = Vec::with_capacity(s.len() / 4 * 3);
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let mut buf = [0i32; 4];
        let mut pad = 0usize;
        for j in 0..4 {
            let c = bytes[i + j];
            buf[j] = match c {
                b'A'..=b'Z' => (c - b'A') as i32,
                b'a'..=b'z' => (c - b'a' + 26) as i32,
                b'0'..=b'9' => (c - b'0' + 52) as i32,
                b'+' => 62,
                b'/' => 63,
                b'=' => {
                    pad += 1;
                    0
                }
                _ => return Err(bad_arg(format!("invalid base64 char: 0x{c:02x}"))),
            };
        }
        let triple = (buf[0] << 18) | (buf[1] << 12) | (buf[2] << 6) | buf[3];
        out.push(((triple >> 16) & 0xff) as u8);
        if pad < 2 {
            out.push(((triple >> 8) & 0xff) as u8);
        }
        if pad < 1 {
            out.push((triple & 0xff) as u8);
        }
        i += 4;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ditto_memory::InMemoryStorage;

    #[test]
    fn server_constructs() {
        let storage = Arc::new(InMemoryStorage::new());
        let key = Arc::new(InstallKey::generate());
        let ctrl = Arc::new(MemoryController::new_with_arc(storage, key));
        let _server = DittoMcpServer::new(ctrl);
    }

    #[test]
    fn parse_slot_accepts_known_slots() {
        assert!(matches!(parse_slot("episodic_index").unwrap(), Slot::EpisodicIndex));
        assert!(matches!(parse_slot("nc_graph").unwrap(), Slot::NcGraph));
        assert!(parse_slot("garbage").is_err());
    }

    #[test]
    fn parse_supersede_accepts_known_policies() {
        assert_eq!(
            parse_supersede("any_with_same_relation").unwrap(),
            SupersedePolicy::AnyWithSameRelation
        );
        assert_eq!(
            parse_supersede("same_src_rel_dst").unwrap(),
            SupersedePolicy::SameSrcRelDst
        );
        assert!(parse_supersede("nonsense").is_err());
    }

    #[test]
    fn parse_ts_round_trip_rfc3339() {
        let t = parse_ts("2026-05-14T12:00:00Z").unwrap();
        assert_eq!(t.to_rfc3339(), "2026-05-14T12:00:00+00:00");
        assert!(parse_ts("not a date").is_err());
    }

    #[test]
    fn b64_encode_decode_round_trips_all_byte_lengths() {
        for n in 0..=20 {
            let raw: Vec<u8> = (0..n).map(|i| (i * 7) as u8).collect();
            let enc = b64_encode(&raw);
            let dec = b64_decode(&enc).unwrap();
            assert_eq!(dec, raw, "round-trip failed at len={n}");
        }
    }

    #[test]
    fn b64_decode_matches_known_strings() {
        // sanity against widely-known fixtures
        assert_eq!(b64_decode("aGVsbG8=").unwrap(), b"hello");
        assert_eq!(b64_decode("aGVsbG8gd29ybGQ=").unwrap(), b"hello world");
        assert!(b64_decode("not%base64=").is_err());
    }
}
