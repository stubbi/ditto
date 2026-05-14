//! Build-body shape tests for `OpenRouterProvider`. These run without HTTP —
//! the point is to lock the wire format against unintended drift.

use ditto_models::provider::openrouter::{
    Attribution, DataCollection, OpenRouterProvider, Precision, RoutingPolicy,
};
use ditto_models::{
    model::{Call, ContentPart, Message, ModelRef, Role},
    tools::{Tool, ToolId, ToolKind},
};
use serde_json::json;
use std::sync::Arc;

fn text_call(model: &str, text: &str) -> Call {
    Call {
        model: ModelRef::new("openrouter", model),
        messages: vec![Message {
            role: Role::User,
            content: vec![ContentPart::Text { text: text.into() }],
        }],
        tools: vec![],
        max_output_tokens: None,
        temperature: None,
        stop: vec![],
        ext: (),
    }
}

#[test]
fn body_contains_required_fields_in_openai_shape() {
    let p = OpenRouterProvider::new();
    let body = p.build_body(&text_call("openai/gpt-5.2", "hi"));

    assert_eq!(body["model"], "openai/gpt-5.2");
    assert_eq!(body["stream"], true);
    assert_eq!(body["stream_options"]["include_usage"], true);
    assert_eq!(body["messages"][0]["role"], "user");
    assert_eq!(body["messages"][0]["content"], "hi");
}

#[test]
fn default_routing_pins_precision_exact_and_denies_data_collection() {
    let p = OpenRouterProvider::new();
    let body = p.build_body(&text_call("anthropic/claude-sonnet-4-6", "x"));
    let routing = &body["provider"];

    // Verified field names per
    // openrouter.ai/docs/guides/routing/provider-selection (2026-05-14).
    assert_eq!(routing["data_collection"], "deny");
    assert_eq!(routing["allow_fallbacks"], true);

    let quants = routing["quantizations"]
        .as_array()
        .expect("quantizations list must be present when Precision::Exact");
    let names: Vec<&str> = quants.iter().filter_map(|v| v.as_str()).collect();
    // Exact == accept fp16/bf16/fp32. The whole point is that fp4/fp6/fp8/int4
    // are *absent* — that's what protects against the CJK-breakage routing.
    assert!(names.contains(&"fp16"));
    assert!(names.contains(&"bf16"));
    assert!(names.contains(&"fp32"));
    for forbidden in ["fp4", "fp6", "fp8", "int4", "int8"] {
        assert!(
            !names.contains(&forbidden),
            "Precision::Exact must not allow {forbidden}"
        );
    }
}

#[test]
fn mixed_precision_does_not_emit_quantizations_field() {
    let p = OpenRouterProvider::new().with_routing(RoutingPolicy {
        precision: Precision::Mixed,
        ..Default::default()
    });
    let body = p.build_body(&text_call("openai/gpt-4o", "x"));
    assert!(body["provider"].get("quantizations").is_none());
}

#[test]
fn data_collection_allow_round_trips_to_wire() {
    let p = OpenRouterProvider::new().with_routing(RoutingPolicy {
        data_collection: DataCollection::Allow,
        ..Default::default()
    });
    let body = p.build_body(&text_call("openai/gpt-4o", "x"));
    assert_eq!(body["provider"]["data_collection"], "allow");
}

#[test]
fn tools_serialize_as_function_calling_shape() {
    let tool = Tool {
        id: ToolId::new("get_weather"),
        kind: ToolKind::Mcp,
        description: "fetch current weather".into(),
        input_schema: json!({
            "type": "object",
            "properties": {"city": {"type": "string"}},
            "required": ["city"],
        }),
        channels: vec![],
    };
    let call = Call {
        model: ModelRef::new("openrouter", "openai/gpt-4o"),
        messages: vec![Message {
            role: Role::User,
            content: vec![ContentPart::Text {
                text: "weather in SF?".into(),
            }],
        }],
        tools: vec![Arc::new(tool)],
        max_output_tokens: None,
        temperature: None,
        stop: vec![],
        ext: (),
    };

    let p = OpenRouterProvider::new();
    let body = p.build_body(&call);
    let t = &body["tools"][0];
    assert_eq!(t["type"], "function");
    assert_eq!(t["function"]["name"], "get_weather");
    assert_eq!(t["function"]["description"], "fetch current weather");
    assert_eq!(t["function"]["parameters"]["required"][0], "city");
}

#[test]
fn tool_call_assistant_messages_round_trip() {
    let call = Call {
        model: ModelRef::new("openrouter", "openai/gpt-4o"),
        messages: vec![
            Message {
                role: Role::Assistant,
                content: vec![ContentPart::ToolCall {
                    id: "call_xyz".into(),
                    name: "get_weather".into(),
                    input: json!({"city": "SF"}),
                }],
            },
            Message {
                role: Role::Tool,
                content: vec![ContentPart::ToolResult {
                    id: "call_xyz".into(),
                    output: json!({"temp": 62}),
                }],
            },
        ],
        tools: vec![],
        max_output_tokens: None,
        temperature: None,
        stop: vec![],
        ext: (),
    };

    let p = OpenRouterProvider::new();
    let body = p.build_body(&call);

    let assistant = &body["messages"][0];
    assert_eq!(assistant["role"], "assistant");
    let tc = &assistant["tool_calls"][0];
    assert_eq!(tc["id"], "call_xyz");
    assert_eq!(tc["type"], "function");
    assert_eq!(tc["function"]["name"], "get_weather");
    // OpenAI specifies tool_call arguments as a *string* (the model writes
    // partial JSON tokens) — adapter must serialize accordingly.
    let args = tc["function"]["arguments"].as_str().unwrap();
    assert_eq!(args, "{\"city\":\"SF\"}");

    let tool_msg = &body["messages"][1];
    assert_eq!(tool_msg["role"], "tool");
    assert_eq!(tool_msg["tool_call_id"], "call_xyz");
    let result_str = tool_msg["content"].as_str().unwrap();
    assert_eq!(result_str, "{\"temp\":62}");
}

#[test]
fn explicit_provider_order_and_ignore_round_trip() {
    let p = OpenRouterProvider::new().with_routing(RoutingPolicy {
        provider_order: vec!["anthropic".into(), "google".into()],
        provider_ignore: vec!["mancer".into()],
        ..Default::default()
    });
    let body = p.build_body(&text_call("anthropic/claude-sonnet-4-6", "hi"));
    let r = &body["provider"];
    assert_eq!(r["order"][0], "anthropic");
    assert_eq!(r["order"][1], "google");
    assert_eq!(r["ignore"][0], "mancer");
}

#[test]
fn attribution_is_attached_via_builder() {
    // Don't actually send a request — just verify the field threads through.
    let _p = OpenRouterProvider::new().with_attribution(Attribution {
        site_url: "https://ditto.example".into(),
        site_title: "ditto-test".into(),
    });
}

#[test]
fn stop_and_temperature_pass_through() {
    let mut call = text_call("openai/gpt-4o", "hi");
    call.temperature = Some(0.2);
    call.max_output_tokens = Some(512);
    call.stop = vec!["END".into(), "STOP".into()];
    let body = OpenRouterProvider::new().build_body(&call);
    let temp = body["temperature"].as_f64().unwrap();
    assert!((temp - 0.2).abs() < 1e-6, "temperature was {temp}");
    assert_eq!(body["max_tokens"], 512);
    assert_eq!(body["stop"][0], "END");
    assert_eq!(body["stop"][1], "STOP");
}
