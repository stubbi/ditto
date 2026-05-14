use futures_core::Stream;
use serde::{Deserialize, Serialize};
use std::pin::Pin;

/// The typed streaming ontology.
///
/// **No `Stream<String>` anywhere.** Streaming SSE deltas back as raw strings
/// is the LiteLLM #20711 / #21331 failure mode — it forces every caller to
/// re-parse provider-specific framing. Every byte that comes off the wire is
/// classified into one of these variants before reaching the agent loop.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    TextDelta {
        text: String,
    },
    /// Reasoning text. Distinct from `TextDelta` so the renderer can hide,
    /// summarize, or persist it separately.
    ReasoningDelta {
        text: String,
        /// Anthropic returns an opaque signature alongside thinking blocks
        /// when extended-thinking is enabled. Pass through verbatim.
        signature: Option<String>,
    },
    /// Synthesized from order-of-arrival when the provider does not expose
    /// `index` natively — the answer to the "every tool call is index 0" bug.
    ToolCallStart {
        id: String,
        name: String,
        index: u32,
    },
    /// JSON-patch fragment for partial tool input. The accumulator is the
    /// caller's responsibility — we never buffer.
    ToolInputDelta {
        id: String,
        json_patch: String,
    },
    ToolInputEnd {
        id: String,
    },
    /// Result of a provider-executed tool (web search, code execution) or a
    /// client-executed tool that has already returned.
    ToolResult {
        id: String,
        content: ToolResultContent,
        provider_executed: bool,
    },
    CacheHit {
        read_tokens: u32,
    },
    CacheWrite {
        write_tokens: u32,
    },
    /// Interim usage update — providers that stream usage mid-call (Anthropic
    /// with `extended-thinking` ramps it up) emit this so cost telemetry can
    /// update without waiting for `Finish`.
    UsageInterim(Usage),
    Finish {
        reason: FinishReason,
        usage: Usage,
    },
    ProviderError {
        code: ErrorClass,
        message: String,
        retryable: bool,
    },
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct Usage {
    pub input_tokens: u32,
    pub cache_read_tokens: u32,
    pub cache_write_tokens: u32,
    pub output_tokens: u32,
    pub reasoning_tokens: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FinishReason {
    Stop,
    Length,
    ToolUse,
    ContentFilter,
    Error,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ToolResultContent {
    Json(serde_json::Value),
    Text(String),
    Image { mime: String, bytes: Vec<u8> },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorClass {
    RateLimit,
    ContextWindowExceeded,
    ContentPolicy,
    Auth,
    Timeout,
    InternalServer,
    NetworkTransient,
}

pub type EventStream = Pin<Box<dyn Stream<Item = Result<Event, crate::Error>> + Send>>;
