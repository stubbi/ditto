//! Tool projection v0: content hashing, dedup, channel filtering.
//!
//! The mode-selection heuristic is not implemented yet (that wants the real
//! token-budget research) but the content-hash and dedup invariants land now
//! because they're what makes Inline-mode bounded at all.

use ditto_models::{
    CachingMode, CapabilitySet, MultimodalCaps, ProjectionMode, RateLimitHeaderShape,
    ReasoningMode, StructuredOutputMode, Tool, ToolCallingShape, ToolId, ToolKind, ToolRegistry,
    TurnProjection,
};
use serde_json::json;

struct DummyCaps;
impl CapabilitySet for DummyCaps {
    fn tool_calling(&self) -> ToolCallingShape {
        ToolCallingShape::OpenAi
    }
    fn prompt_caching(&self) -> CachingMode {
        CachingMode::None
    }
    fn reasoning(&self) -> ReasoningMode {
        ReasoningMode::None
    }
    fn multimodal(&self) -> MultimodalCaps {
        MultimodalCaps::default()
    }
    fn batching(&self) -> bool {
        false
    }
    fn native_web_search(&self) -> bool {
        false
    }
    fn rate_limit_headers(&self) -> RateLimitHeaderShape {
        RateLimitHeaderShape::None
    }
    fn structured_outputs(&self) -> StructuredOutputMode {
        StructuredOutputMode::None
    }
}

fn mk_tool(id: &str, schema: serde_json::Value, channels: Vec<&str>) -> Tool {
    Tool {
        id: ToolId::new(id),
        kind: ToolKind::Mcp,
        description: format!("tool {id}"),
        input_schema: schema,
        channels: channels.into_iter().map(String::from).collect(),
    }
}

#[test]
fn schema_hash_is_canonical_json_invariant() {
    // Two tools with the same logical schema but different key order should
    // produce the same SchemaHash. That's the property that makes caching
    // across turns and across providers work.
    let a = mk_tool("a", json!({"type": "object", "required": ["q"]}), vec![]);
    let b = mk_tool("b", json!({"required": ["q"], "type": "object"}), vec![]);
    assert_eq!(a.schema_hash().unwrap(), b.schema_hash().unwrap());
}

#[test]
fn registry_dedupes_identical_schemas() {
    let mut reg = ToolRegistry::new();
    reg.insert(mk_tool("a", json!({"type": "object"}), vec![]))
        .unwrap();
    reg.insert(mk_tool("b", json!({"type": "object"}), vec![]))
        .unwrap();
    reg.insert(mk_tool("c", json!({"type": "string"}), vec![]))
        .unwrap();

    let caps = DummyCaps;
    let proj = reg.project(TurnProjection {
        channel: None,
        allowed_kinds: &[ToolKind::Mcp],
        budget_tokens: 8_000,
        mode: ProjectionMode::Inline,
        provider_caps: &caps,
    });

    assert_eq!(proj.tools.len(), 3);
    // Two distinct schemas across three tools.
    assert_eq!(proj.schemas.len(), 2);
}

#[test]
fn channel_filter_excludes_unauthorized_tools() {
    let mut reg = ToolRegistry::new();
    reg.insert(mk_tool(
        "browser_open",
        json!({"type": "object"}),
        vec!["cli"],
    ))
    .unwrap();
    reg.insert(mk_tool(
        "say_hi",
        json!({"type": "object"}),
        vec!["telegram", "cli"],
    ))
    .unwrap();

    let caps = DummyCaps;
    let proj = reg.project(TurnProjection {
        channel: Some("telegram"),
        allowed_kinds: &[ToolKind::Mcp],
        budget_tokens: 8_000,
        mode: ProjectionMode::Inline,
        provider_caps: &caps,
    });

    assert_eq!(proj.tools.len(), 1);
    assert_eq!(proj.tools[0].0, "say_hi");
}

#[test]
fn kind_filter_excludes_provider_native_when_not_requested() {
    let mut reg = ToolRegistry::new();
    reg.insert(Tool {
        id: ToolId::new("anthropic_web_search"),
        kind: ToolKind::ProviderNative,
        description: "provider-side web search".into(),
        input_schema: json!({"type": "object"}),
        channels: vec![],
    })
    .unwrap();
    reg.insert(mk_tool("mcp_tool", json!({"type": "object"}), vec![]))
        .unwrap();

    let caps = DummyCaps;
    let proj = reg.project(TurnProjection {
        channel: None,
        allowed_kinds: &[ToolKind::Mcp],
        budget_tokens: 8_000,
        mode: ProjectionMode::Inline,
        provider_caps: &caps,
    });

    assert_eq!(proj.tools.len(), 1);
    assert_eq!(proj.tools[0].0, "mcp_tool");
}

#[test]
fn schema_hash_hex_is_64_chars() {
    let t = mk_tool("a", json!({"type": "object"}), vec![]);
    assert_eq!(t.schema_hash().unwrap().hex().len(), 64);
}

#[test]
fn search_mode_round_trips_through_serde() {
    let m = ProjectionMode::Search {
        index_tool: ToolId::new("tools.search"),
    };
    let j = serde_json::to_string(&m).unwrap();
    assert!(j.contains("\"kind\":\"search\""));
    let _: ProjectionMode = serde_json::from_str(&j).unwrap();
}

#[test]
fn code_execution_mode_round_trips_through_serde() {
    let m = ProjectionMode::CodeExecution {
        entrypoint_module: "ditto_tools".into(),
    };
    let j = serde_json::to_string(&m).unwrap();
    assert!(j.contains("\"kind\":\"code_execution\""));
    let _: ProjectionMode = serde_json::from_str(&j).unwrap();
}
