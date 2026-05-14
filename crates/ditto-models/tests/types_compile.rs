//! Smoke tests for the v0 type ontology. These exist to lock the trait
//! surface — if a future change breaks them, every adapter author needs to
//! sign off on the breakage.

use ditto_models::{
    CachingMode, CallCost, CapabilitySet, CostBreakdown, ErrorClass, Event, FinishReason,
    ModelDescriptor, ModelRef, MultimodalCaps, PolicyStatus, ProviderId, RateLimitClass,
    RateLimitHeaderShape, ReasoningMode, Region, StructuredOutputMode, ToolCallingShape, Usage,
};

#[test]
fn provider_id_round_trips_string() {
    let p = ProviderId::new("openrouter");
    assert_eq!(p.as_str(), "openrouter");
    assert_eq!(p.to_string(), "openrouter");
}

#[test]
fn model_ref_constructs_without_snapshot() {
    let m = ModelRef::new("anthropic", "claude-sonnet-4-6");
    assert_eq!(m.provider.as_str(), "anthropic");
    assert_eq!(m.model, "claude-sonnet-4-6");
    assert!(m.snapshot.is_none());
}

#[test]
fn cost_breakdown_total_is_sum_of_lines() {
    let cb = CostBreakdown {
        input_usd: 1.0,
        cache_read_usd: 0.1,
        cache_write_usd: 1.25,
        output_usd: 2.0,
        reasoning_usd: 0.5,
        openrouter_fee_usd: 0.05,
    };
    assert!((cb.total() - 4.90).abs() < 1e-9);
}

#[test]
fn cache_lines_round_trip_through_call_cost() {
    let c = CallCost {
        input_tokens: 100,
        cache_read_tokens: 200,
        cache_write_tokens: 50,
        output_tokens: 300,
        reasoning_tokens: 400,
        usd: 12.34,
        usd_breakdown: CostBreakdown::default(),
    };
    let json = serde_json::to_string(&c).unwrap();
    let back: CallCost = serde_json::from_str(&json).unwrap();
    assert_eq!(c, back);
}

#[test]
fn event_round_trips_through_json() {
    let e = Event::ReasoningDelta {
        text: "thinking about it".into(),
        signature: Some("opaque-sig".into()),
    };
    let json = serde_json::to_string(&e).unwrap();
    assert!(json.contains("\"type\":\"reasoning_delta\""));
    let _: Event = serde_json::from_str(&json).unwrap();
}

#[test]
fn finish_reason_variants_serialize_snake_case() {
    let j = serde_json::to_string(&FinishReason::ToolUse).unwrap();
    assert_eq!(j, "\"tool_use\"");
}

#[test]
fn error_class_round_trips() {
    let e = ErrorClass::ContextWindowExceeded;
    let j = serde_json::to_string(&e).unwrap();
    assert_eq!(j, "\"context_window_exceeded\"");
    let back: ErrorClass = serde_json::from_str(&j).unwrap();
    assert_eq!(e, back);
}

#[test]
fn policy_status_includes_enforced_block() {
    // The whole reason this enum exists is so EnforcedBlock is callable from
    // user code. The architecture commits to refusing Claude Code OAuth by
    // default post-2026-04-04 — that surfaces here.
    let _ = PolicyStatus::EnforcedBlock;
    let _ = PolicyStatus::GreyArea;
    let _ = PolicyStatus::Allowed;
}

#[test]
fn capability_set_can_be_implemented_as_a_const_struct() {
    // Adapter authors typically implement CapabilitySet as a zero-sized type
    // returning constants. This test pins that pattern.
    struct OpenRouterCaps;
    impl CapabilitySet for OpenRouterCaps {
        fn tool_calling(&self) -> ToolCallingShape {
            ToolCallingShape::OpenAi
        }
        fn prompt_caching(&self) -> CachingMode {
            // OpenRouter exposes Anthropic cache_control only when routed to
            // Anthropic backends — Implicit is the safe default.
            CachingMode::Implicit
        }
        fn reasoning(&self) -> ReasoningMode {
            ReasoningMode::Hidden
        }
        fn multimodal(&self) -> MultimodalCaps {
            MultimodalCaps {
                image_input: true,
                ..Default::default()
            }
        }
        fn batching(&self) -> bool {
            false
        }
        fn native_web_search(&self) -> bool {
            false
        }
        fn rate_limit_headers(&self) -> RateLimitHeaderShape {
            RateLimitHeaderShape::OpenAi
        }
        fn structured_outputs(&self) -> StructuredOutputMode {
            StructuredOutputMode::JsonMode
        }
    }
    let c = OpenRouterCaps;
    assert_eq!(c.tool_calling(), ToolCallingShape::OpenAi);
    assert_eq!(c.prompt_caching(), CachingMode::Implicit);
}

#[test]
fn rate_limit_class_and_region_are_round_trippable() {
    let j = serde_json::to_string(&RateLimitClass::Paid).unwrap();
    let _: RateLimitClass = serde_json::from_str(&j).unwrap();
    let j2 = serde_json::to_string(&Region::EuCentral).unwrap();
    let _: Region = serde_json::from_str(&j2).unwrap();
}

#[test]
fn model_descriptor_holds_deprecation_flag() {
    let md = ModelDescriptor {
        id: "gpt-4o-2024-05-13".into(),
        display_name: "GPT-4o (May 2024)".into(),
        context_window: 128_000,
        max_output_tokens: Some(16_384),
        knowledge_cutoff: None,
        deprecated: true,
    };
    assert!(md.deprecated);
}

#[test]
fn usage_default_is_zero() {
    let u = Usage::default();
    assert_eq!(u.input_tokens, 0);
    assert_eq!(u.cache_read_tokens, 0);
}
