//! SSE parsing tests for `OpenRouterProvider`. These run against recorded
//! byte fixtures that mirror what OpenRouter actually emits per
//! `openrouter.ai/docs/api/reference/streaming` (verified 2026-05-14).
//!
//! Hitting the real API in unit tests would be flaky and bill the user. The
//! recorded-fixture approach matches what Vercel AI SDK and async-openai do.

use ditto_models::provider::openrouter::__test_parse_sse;
use ditto_models::stream::{Event, FinishReason};

#[test]
fn text_delta_stream_parses_into_finish() {
    let raw = concat!(
        "data: {\"id\":\"x\",\"object\":\"chat.completion.chunk\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"Hello\"}}]}\n\n",
        "data: {\"id\":\"x\",\"object\":\"chat.completion.chunk\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\" world\"}}]}\n\n",
        "data: {\"id\":\"x\",\"object\":\"chat.completion.chunk\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":10,\"completion_tokens\":2,\"total_tokens\":12}}\n\n",
        "data: [DONE]\n\n",
    );

    let events = __test_parse_sse(raw.as_bytes()).unwrap();
    let mut text = String::new();
    let mut got_finish = false;
    for ev in events {
        match ev {
            Event::TextDelta { text: t } => text.push_str(&t),
            Event::Finish { reason, usage } => {
                assert_eq!(reason, FinishReason::Stop);
                assert_eq!(usage.input_tokens, 10);
                assert_eq!(usage.output_tokens, 2);
                got_finish = true;
            }
            _ => {}
        }
    }
    assert_eq!(text, "Hello world");
    assert!(got_finish);
}

#[test]
fn comment_lines_are_ignored() {
    // OpenRouter sends ": OPENROUTER PROCESSING" comments to keep the
    // connection alive. They must not produce typed events.
    let raw = concat!(
        ": OPENROUTER PROCESSING\n\n",
        "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"x\"}}]}\n\n",
        "data: [DONE]\n\n",
    );
    let events = __test_parse_sse(raw.as_bytes()).unwrap();
    assert_eq!(events.len(), 1);
    matches!(events[0], Event::TextDelta { .. });
}

#[test]
fn reasoning_field_maps_to_reasoning_delta() {
    let raw = concat!(
        "data: {\"choices\":[{\"index\":0,\"delta\":{\"reasoning\":\"thinking about it\"}}]}\n\n",
        "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"42\"},\"finish_reason\":\"stop\"}]}\n\n",
        "data: [DONE]\n\n",
    );
    let events = __test_parse_sse(raw.as_bytes()).unwrap();
    let kinds: Vec<&str> = events
        .iter()
        .map(|e| match e {
            Event::TextDelta { .. } => "text",
            Event::ReasoningDelta { .. } => "reasoning",
            Event::Finish { .. } => "finish",
            _ => "other",
        })
        .collect();
    assert_eq!(kinds, vec!["reasoning", "text", "finish"]);
}

#[test]
fn tool_call_streaming_emits_start_then_input_deltas_then_end() {
    // Mirrors how OpenAI Chat Completions streams a single tool call:
    // chunk A names the call (id + name), chunk B-D append partial arguments,
    // chunk E carries finish_reason=tool_calls.
    let raw = concat!(
        "data: {\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_abc\",\"type\":\"function\",\"function\":{\"name\":\"get_weather\",\"arguments\":\"\"}}]}}]}\n\n",
        "data: {\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"{\\\"c\"}}]}}]}\n\n",
        "data: {\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"ity\\\":\\\"SF\\\"}\"}}]}}]}\n\n",
        "data: {\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"tool_calls\"}],\"usage\":{\"prompt_tokens\":50,\"completion_tokens\":12,\"total_tokens\":62}}\n\n",
        "data: [DONE]\n\n",
    );

    let events = __test_parse_sse(raw.as_bytes()).unwrap();

    // First event: ToolCallStart with index=0, id=call_abc, name=get_weather.
    match &events[0] {
        Event::ToolCallStart { id, name, index } => {
            assert_eq!(id, "call_abc");
            assert_eq!(name, "get_weather");
            assert_eq!(*index, 0);
        }
        e => panic!("expected ToolCallStart, got {e:?}"),
    }

    // Concatenate all ToolInputDelta json_patch fragments and confirm we
    // assembled the full tool-input string.
    let mut args = String::new();
    let mut finish = None;
    let mut tool_input_end = false;
    for e in events.iter().skip(1) {
        match e {
            Event::ToolInputDelta { id, json_patch } => {
                assert_eq!(id, "call_abc");
                args.push_str(json_patch);
            }
            Event::ToolInputEnd { id } => {
                assert_eq!(id, "call_abc");
                tool_input_end = true;
            }
            Event::Finish { reason, usage } => {
                finish = Some((reason, usage.clone()));
            }
            _ => {}
        }
    }
    assert_eq!(args, "{\"city\":\"SF\"}");
    assert!(tool_input_end, "ToolInputEnd must close every started call");
    let (reason, usage) = finish.expect("finish event must arrive");
    assert_eq!(*reason, FinishReason::ToolUse);
    assert_eq!(usage.input_tokens, 50);
    assert_eq!(usage.output_tokens, 12);
}

#[test]
fn chunks_split_across_byte_boundaries_still_parse() {
    // OpenRouter's TCP framing can fragment chunks mid-line. The parser must
    // buffer until it sees `\n\n` — this fixture deliberately splits the
    // first chunk between two reads to verify that.
    let part1 = b"data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"Hel";
    let part2 = b"lo\"}}]}\n\ndata: [DONE]\n\n";
    let mut joined = Vec::new();
    joined.extend_from_slice(part1);
    joined.extend_from_slice(part2);
    let events = __test_parse_sse(&joined).unwrap();
    assert_eq!(events.len(), 1);
    matches!(&events[0], Event::TextDelta { text } if text == "Hello");
}

#[test]
fn cached_tokens_in_usage_map_to_cache_read() {
    let raw = "data: {\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":100,\"completion_tokens\":5,\"total_tokens\":105,\"prompt_tokens_details\":{\"cached_tokens\":80},\"completion_tokens_details\":{\"reasoning_tokens\":3}}}\n\ndata: [DONE]\n\n";
    let events = __test_parse_sse(raw.as_bytes()).unwrap();
    let finish = events
        .iter()
        .find_map(|e| match e {
            Event::Finish { usage, .. } => Some(usage.clone()),
            _ => None,
        })
        .unwrap();
    assert_eq!(finish.cache_read_tokens, 80);
    assert_eq!(finish.reasoning_tokens, 3);
}
