//! OpenRouter — Ditto's default BYOK catalog adapter.
//!
//! Why this is the first concrete `Provider`: 200+ models behind one
//! OpenAI-shape endpoint forces the trait surface against the maximum amount
//! of provider variance in a single integration. If the typed `Event`
//! ontology and `CapabilitySet` can serve OpenRouter, they can serve every
//! native-API adapter that comes after.
//!
//! Wire format verified 2026-05-14 against
//! `openrouter.ai/docs/api/reference/streaming` and
//! `openrouter.ai/docs/guides/routing/provider-selection`. Authentication
//! header (`X-OpenRouter-Title`, not the older `X-Title`) verified against
//! `openrouter.ai/docs/api/reference/authentication`.
//!
//! Defaults that matter:
//! - `Precision::Exact` is on by default. This maps to a
//!   `provider.quantizations` list excluding `int4`/`int8`/`fp4`/`fp6`/`fp8`,
//!   so OpenRouter's default routing to FP4/Int4 backends (the
//!   CJK-encoding-breaks-on-quantization issue documented in RooCodeInc
//!   #11325 and QwenLM/qwen-code #348) can't silently degrade output quality.
//! - Streaming surfaces a usage block via `stream_options.include_usage`, so
//!   cost telemetry doesn't have to wait until the connection closes.

use crate::auth::AuthHandle;
use crate::capabilities::{
    CachingMode, CapabilitySet, MultimodalCaps, RateLimitHeaderShape, ReasoningMode,
    StructuredOutputMode, ToolCallingShape,
};
use crate::model::{Call, ContentPart, Message, ModelDescriptor, ProviderId, Role};
use crate::stream::{ErrorClass, Event, EventStream, FinishReason, Usage};
use crate::tools::Tool;
use crate::Error;
use async_trait::async_trait;
use bytes::BytesMut;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;

pub const PROVIDER_ID: &str = "openrouter";
pub const DEFAULT_BASE_URL: &str = "https://openrouter.ai/api/v1";

#[derive(Clone)]
pub struct OpenRouterProvider {
    base_url: String,
    http: reqwest::Client,
    attribution: Option<Attribution>,
    routing: RoutingPolicy,
    models: Arc<Vec<ModelDescriptor>>,
}

/// Site attribution headers — `HTTP-Referer` + `X-OpenRouter-Title`. Optional
/// per OpenRouter's auth docs, but populating them is the only way the user
/// shows up on the rankings dashboard.
#[derive(Clone, Debug, Default)]
pub struct Attribution {
    pub site_url: String,
    pub site_title: String,
}

#[derive(Clone, Debug)]
pub struct RoutingPolicy {
    pub precision: Precision,
    pub allow_fallbacks: bool,
    pub data_collection: DataCollection,
    pub require_parameters: bool,
    pub provider_order: Vec<String>,
    pub provider_ignore: Vec<String>,
}

impl Default for RoutingPolicy {
    fn default() -> Self {
        Self {
            precision: Precision::Exact,
            allow_fallbacks: true,
            // Default to `deny` rather than `allow`. The Bet 3 wedge is built
            // on the user's trust that their prompts aren't being harvested
            // for training. Opt-in if explicitly configured.
            data_collection: DataCollection::Deny,
            require_parameters: false,
            provider_order: Vec::new(),
            provider_ignore: Vec::new(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Precision {
    /// Refuse FP4 / FP6 / FP8 / Int4 / Int8 routing. Maps to a
    /// `quantizations` list of `["fp16", "bf16", "fp32"]`.
    Exact,
    /// Accept whatever OpenRouter picks. Cheaper, but the CJK-breakage and
    /// quality-drift complaints in the OSS research apply.
    Mixed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DataCollection {
    Allow,
    Deny,
}

impl OpenRouterProvider {
    pub fn new() -> Self {
        Self {
            base_url: DEFAULT_BASE_URL.into(),
            http: reqwest::Client::new(),
            attribution: None,
            routing: RoutingPolicy::default(),
            models: Arc::new(Vec::new()),
        }
    }

    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = url.into();
        self
    }

    pub fn with_attribution(mut self, attribution: Attribution) -> Self {
        self.attribution = Some(attribution);
        self
    }

    pub fn with_routing(mut self, routing: RoutingPolicy) -> Self {
        self.routing = routing;
        self
    }

    pub fn with_http_client(mut self, http: reqwest::Client) -> Self {
        self.http = http;
        self
    }

    pub fn with_models(mut self, models: Vec<ModelDescriptor>) -> Self {
        self.models = Arc::new(models);
        self
    }

    /// Build the on-the-wire JSON body. Pure and synchronous so it can be
    /// unit-tested against expected snapshots without spinning up an HTTP
    /// mock.
    pub fn build_body(&self, call: &Call) -> Value {
        let mut messages = Vec::with_capacity(call.messages.len());
        for m in &call.messages {
            messages.extend(message_to_wire(m));
        }

        let tools: Vec<Value> = call.tools.iter().map(|t| tool_to_wire(t)).collect();

        let mut body = json!({
            "model": call.model.model,
            "messages": messages,
            "stream": true,
            "stream_options": { "include_usage": true },
        });

        if !tools.is_empty() {
            body["tools"] = Value::Array(tools);
        }
        if let Some(t) = call.temperature {
            body["temperature"] = json!(t);
        }
        if let Some(m) = call.max_output_tokens {
            body["max_tokens"] = json!(m);
        }
        if !call.stop.is_empty() {
            body["stop"] = json!(call.stop);
        }

        body["provider"] = provider_routing_to_wire(&self.routing);
        body
    }
}

impl Default for OpenRouterProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl crate::provider::Provider for OpenRouterProvider {
    type Capabilities = OpenRouterCaps;

    fn id(&self) -> ProviderId {
        ProviderId::new(PROVIDER_ID)
    }

    fn models(&self) -> &[ModelDescriptor] {
        &self.models
    }

    fn capabilities(&self) -> Self::Capabilities {
        OpenRouterCaps
    }

    async fn stream(&self, call: Call, auth: &AuthHandle) -> Result<EventStream, Error> {
        let token = auth.token().await?;
        let body = self.build_body(&call);

        let mut req = self
            .http
            .post(format!("{}/chat/completions", self.base_url))
            .bearer_auth(&token.token)
            .header("Content-Type", "application/json");

        if let Some(a) = &self.attribution {
            req = req
                .header("HTTP-Referer", &a.site_url)
                .header("X-OpenRouter-Title", &a.site_title);
        }
        for (k, v) in &token.aux_headers {
            req = req.header(k, v);
        }

        let resp = req.json(&body).send().await.map_err(reqwest_to_error)?;
        let status = resp.status();
        if !status.is_success() {
            let bytes = resp.bytes().await.unwrap_or_default();
            return Err(http_error(status, &bytes));
        }

        let mut stream = resp.bytes_stream();
        let s = async_stream::try_stream! {
            let mut buf = BytesMut::new();
            let mut tool_call_ids: Vec<Option<String>> = Vec::new();

            while let Some(chunk) = stream.next().await {
                let chunk = chunk.map_err(reqwest_to_error)?;
                buf.extend_from_slice(&chunk);

                while let Some(end) = find_event_boundary(&buf) {
                    let raw = buf.split_to(end + 2);
                    let text = match std::str::from_utf8(&raw) {
                        Ok(s) => s,
                        Err(_) => continue,
                    };
                    for line in text.split('\n') {
                        let line = line.trim_end_matches('\r');
                        if line.is_empty() || line.starts_with(':') {
                            continue;
                        }
                        let Some(data) = line.strip_prefix("data: ").or_else(|| line.strip_prefix("data:"))
                        else {
                            continue;
                        };
                        if data.trim() == "[DONE]" {
                            return;
                        }
                        match serde_json::from_str::<ChatChunk>(data) {
                            Ok(chunk_json) => {
                                for event in map_chunk(chunk_json, &mut tool_call_ids) {
                                    yield event;
                                }
                            }
                            Err(e) => {
                                tracing::warn!(error = %e, payload = data, "openrouter: chunk parse failed");
                            }
                        }
                    }
                }
            }
        };

        Ok(Box::pin(s))
    }
}

/// Provider capabilities for OpenRouter routed to a generic upstream.
///
/// Conservative defaults — OpenRouter passes through capabilities from the
/// concrete upstream model, but at the routing layer we can only promise the
/// intersection. Per-upstream caps belong on the model descriptor, not here.
pub struct OpenRouterCaps;

impl CapabilitySet for OpenRouterCaps {
    fn tool_calling(&self) -> ToolCallingShape {
        ToolCallingShape::OpenAi
    }
    fn prompt_caching(&self) -> CachingMode {
        // OpenRouter passes Anthropic cache_control through to upstream
        // Anthropic backends only. At the routing-layer level the safe
        // promise is Implicit; opt-in to ExplicitCacheControl per model.
        CachingMode::Implicit
    }
    fn reasoning(&self) -> ReasoningMode {
        // Surfaces `delta.reasoning` for upstreams that support it (Anthropic
        // extended-thinking, DeepSeek). We pass that through as
        // ReasoningDelta — see `map_chunk`.
        ReasoningMode::ExplicitBlocks
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

// =============== wire <-> typed conversions ===============

fn message_to_wire(m: &Message) -> Vec<Value> {
    // OpenAI shape: a message with role + content (string or content-parts).
    // Tool results are their own role=tool messages keyed by tool_call_id.
    let role = match m.role {
        Role::System => "system",
        Role::User => "user",
        Role::Assistant => "assistant",
        Role::Tool => "tool",
    };

    let mut tool_results: Vec<Value> = Vec::new();
    let mut content_parts: Vec<Value> = Vec::new();
    let mut tool_calls: Vec<Value> = Vec::new();

    for part in &m.content {
        match part {
            ContentPart::Text { text } => {
                content_parts.push(json!({"type": "text", "text": text}));
            }
            ContentPart::Image { mime, bytes } => {
                let data_url = format!("data:{};base64,{}", mime, base64_encode(bytes));
                content_parts.push(json!({
                    "type": "image_url",
                    "image_url": { "url": data_url },
                }));
            }
            ContentPart::ToolCall { id, name, input } => {
                tool_calls.push(json!({
                    "id": id,
                    "type": "function",
                    "function": {
                        "name": name,
                        "arguments": serde_json::to_string(input).unwrap_or_default(),
                    },
                }));
            }
            ContentPart::ToolResult { id, output } => {
                tool_results.push(json!({
                    "role": "tool",
                    "tool_call_id": id,
                    "content": serde_json::to_string(output).unwrap_or_default(),
                }));
            }
        }
    }

    let mut out = Vec::new();
    if !content_parts.is_empty() || !tool_calls.is_empty() {
        let mut msg = json!({ "role": role });
        if content_parts.len() == 1 {
            if let Some(text) = content_parts[0].get("text").and_then(|v| v.as_str()) {
                msg["content"] = json!(text);
            } else {
                msg["content"] = Value::Array(content_parts);
            }
        } else if !content_parts.is_empty() {
            msg["content"] = Value::Array(content_parts);
        }
        if !tool_calls.is_empty() {
            msg["tool_calls"] = Value::Array(tool_calls);
        }
        out.push(msg);
    }
    out.extend(tool_results);
    out
}

fn tool_to_wire(t: &Tool) -> Value {
    json!({
        "type": "function",
        "function": {
            "name": t.id.0,
            "description": t.description,
            "parameters": t.input_schema,
        },
    })
}

fn provider_routing_to_wire(r: &RoutingPolicy) -> Value {
    let mut obj = serde_json::Map::new();
    obj.insert("allow_fallbacks".into(), json!(r.allow_fallbacks));
    obj.insert(
        "data_collection".into(),
        match r.data_collection {
            DataCollection::Allow => json!("allow"),
            DataCollection::Deny => json!("deny"),
        },
    );
    obj.insert("require_parameters".into(), json!(r.require_parameters));
    if matches!(r.precision, Precision::Exact) {
        // Verified options per openrouter.ai/docs/guides/routing/provider-selection
        // (2026-05-14): int4, int8, fp4, fp6, fp8, fp16, bf16, fp32, unknown.
        // "Exact" = no lossy quant — fp16 / bf16 / fp32 only.
        obj.insert(
            "quantizations".into(),
            json!(["fp16", "bf16", "fp32"]),
        );
    }
    if !r.provider_order.is_empty() {
        obj.insert("order".into(), json!(r.provider_order));
    }
    if !r.provider_ignore.is_empty() {
        obj.insert("ignore".into(), json!(r.provider_ignore));
    }
    Value::Object(obj)
}

// =============== streaming chunk -> typed Event ===============

#[derive(Debug, Deserialize)]
struct ChatChunk {
    #[serde(default)]
    choices: Vec<ChatChoice>,
    #[serde(default)]
    usage: Option<UsageWire>,
    #[serde(default)]
    error: Option<ErrorWire>,
}

#[derive(Debug, Deserialize)]
struct ChatChoice {
    #[serde(default)]
    delta: ChoiceDelta,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct ChoiceDelta {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    reasoning: Option<String>,
    #[serde(default)]
    tool_calls: Vec<ToolCallDelta>,
}

#[derive(Debug, Deserialize)]
struct ToolCallDelta {
    #[serde(default)]
    index: Option<u32>,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    function: Option<FunctionDelta>,
}

#[derive(Debug, Deserialize)]
struct FunctionDelta {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    arguments: Option<String>,
}

#[derive(Debug, Default, Deserialize, Serialize)]
struct UsageWire {
    #[serde(default)]
    prompt_tokens: u32,
    #[serde(default)]
    completion_tokens: u32,
    #[serde(default)]
    total_tokens: u32,
    #[serde(default)]
    prompt_tokens_details: Option<PromptTokensDetails>,
    #[serde(default)]
    completion_tokens_details: Option<CompletionTokensDetails>,
}

#[derive(Debug, Default, Deserialize, Serialize)]
struct PromptTokensDetails {
    #[serde(default)]
    cached_tokens: u32,
}

#[derive(Debug, Default, Deserialize, Serialize)]
struct CompletionTokensDetails {
    #[serde(default)]
    reasoning_tokens: u32,
}

#[derive(Debug, Deserialize)]
struct ErrorWire {
    code: Option<serde_json::Value>,
    message: Option<String>,
}

fn map_chunk(chunk: ChatChunk, tool_call_ids: &mut Vec<Option<String>>) -> Vec<Event> {
    let mut out = Vec::new();

    if let Some(err) = chunk.error {
        out.push(Event::ProviderError {
            code: classify_error(&err),
            message: err.message.unwrap_or_default(),
            // OpenRouter mid-stream errors are usually upstream rate-limits or
            // routing failures — caller decides on retry policy from the
            // typed class, not this hint.
            retryable: false,
        });
    }

    for choice in chunk.choices {
        if let Some(text) = choice.delta.content {
            if !text.is_empty() {
                out.push(Event::TextDelta { text });
            }
        }
        if let Some(reasoning) = choice.delta.reasoning {
            if !reasoning.is_empty() {
                out.push(Event::ReasoningDelta {
                    text: reasoning,
                    signature: None,
                });
            }
        }
        for tc in choice.delta.tool_calls {
            let idx = tc.index.unwrap_or(0) as usize;
            while tool_call_ids.len() <= idx {
                tool_call_ids.push(None);
            }
            if let (Some(id), Some(func)) = (tc.id.as_ref(), tc.function.as_ref()) {
                if let Some(name) = func.name.as_ref() {
                    tool_call_ids[idx] = Some(id.clone());
                    out.push(Event::ToolCallStart {
                        id: id.clone(),
                        name: name.clone(),
                        index: idx as u32,
                    });
                }
            } else if let Some(id) = tc.id.as_ref() {
                tool_call_ids[idx] = Some(id.clone());
            }
            if let Some(func) = tc.function.as_ref() {
                if let Some(args) = func.arguments.as_ref() {
                    if !args.is_empty() {
                        if let Some(Some(id)) = tool_call_ids.get(idx) {
                            out.push(Event::ToolInputDelta {
                                id: id.clone(),
                                json_patch: args.clone(),
                            });
                        }
                    }
                }
            }
        }
        if let Some(reason) = choice.finish_reason {
            // Close out any open tool inputs first — every ToolCallStart needs
            // a matching ToolInputEnd before Finish for clean accounting.
            for id in tool_call_ids.drain(..).flatten() {
                out.push(Event::ToolInputEnd { id });
            }
            let usage = chunk
                .usage
                .as_ref()
                .map(usage_to_typed)
                .unwrap_or_default();
            out.push(Event::Finish {
                reason: classify_finish(&reason),
                usage,
            });
            return out;
        }
    }

    if let Some(usage) = chunk.usage.as_ref() {
        let typed = usage_to_typed(usage);
        if typed.input_tokens > 0 || typed.output_tokens > 0 {
            // OpenRouter sometimes emits a usage-only chunk after the final
            // choice has already produced a Finish (when the connection is
            // still open for a moment after). Surface it as UsageInterim so
            // cost accounting can fold it in without producing a second
            // Finish.
            out.push(Event::UsageInterim(typed));
        }
    }

    out
}

fn usage_to_typed(u: &UsageWire) -> Usage {
    Usage {
        input_tokens: u.prompt_tokens,
        cache_read_tokens: u
            .prompt_tokens_details
            .as_ref()
            .map(|d| d.cached_tokens)
            .unwrap_or(0),
        cache_write_tokens: 0,
        output_tokens: u.completion_tokens,
        reasoning_tokens: u
            .completion_tokens_details
            .as_ref()
            .map(|d| d.reasoning_tokens)
            .unwrap_or(0),
    }
}

fn classify_finish(reason: &str) -> FinishReason {
    match reason {
        "stop" | "end_turn" => FinishReason::Stop,
        "length" | "max_tokens" => FinishReason::Length,
        "tool_calls" | "tool_use" | "function_call" => FinishReason::ToolUse,
        "content_filter" | "safety" => FinishReason::ContentFilter,
        _ => FinishReason::Stop,
    }
}

fn classify_error(err: &ErrorWire) -> ErrorClass {
    let code = err
        .code
        .as_ref()
        .and_then(|c| c.as_u64())
        .or_else(|| err.code.as_ref().and_then(|c| c.as_str()).and_then(|s| s.parse().ok()));
    match code {
        Some(429) => ErrorClass::RateLimit,
        Some(401) | Some(403) => ErrorClass::Auth,
        Some(408) => ErrorClass::Timeout,
        Some(c) if (500..600).contains(&c) => ErrorClass::InternalServer,
        _ => {
            // Anthropic-style upstream context-window messages get classified
            // even without a numeric code.
            let msg = err.message.as_deref().unwrap_or("").to_ascii_lowercase();
            if msg.contains("context") && msg.contains("length") {
                ErrorClass::ContextWindowExceeded
            } else {
                ErrorClass::InternalServer
            }
        }
    }
}

// =============== SSE framing + utility ===============

fn find_event_boundary(buf: &BytesMut) -> Option<usize> {
    buf.windows(2).position(|w| w == b"\n\n")
}

fn reqwest_to_error(e: reqwest::Error) -> Error {
    Error::Other(anyhow::anyhow!(e))
}

fn http_error(status: reqwest::StatusCode, body: &[u8]) -> Error {
    let text = std::str::from_utf8(body).unwrap_or("<non-utf8 body>");
    Error::Other(anyhow::anyhow!("openrouter http {}: {}", status, text))
}

fn base64_encode(bytes: &[u8]) -> String {
    // Tiny inline base64 — image payloads are rare in v0 and pulling a base64
    // crate just for this is overkill. Standard alphabet, no padding tricks.
    const ALPH: &[u8; 64] =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            if chunk.len() > 1 { chunk[1] } else { 0 },
            if chunk.len() > 2 { chunk[2] } else { 0 },
        ];
        out.push(ALPH[(b[0] >> 2) as usize] as char);
        out.push(ALPH[((b[0] & 0x03) << 4 | b[1] >> 4) as usize] as char);
        if chunk.len() > 1 {
            out.push(ALPH[((b[1] & 0x0f) << 2 | b[2] >> 6) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(ALPH[(b[2] & 0x3f) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

/// Test-only access: parse a recorded SSE byte stream into typed Events. Used
/// by integration tests that record fixtures rather than hitting live OpenRouter.
#[doc(hidden)]
pub fn __test_parse_sse(bytes: &[u8]) -> Result<Vec<Event>, Error> {
    let mut buf = BytesMut::from(bytes);
    let mut tool_call_ids: Vec<Option<String>> = Vec::new();
    let mut events = Vec::new();
    while let Some(end) = find_event_boundary(&buf) {
        let raw = buf.split_to(end + 2);
        let text = std::str::from_utf8(&raw)
            .map_err(|e| Error::Other(anyhow::anyhow!("utf8: {e}")))?;
        for line in text.split('\n') {
            let line = line.trim_end_matches('\r');
            if line.is_empty() || line.starts_with(':') {
                continue;
            }
            let Some(data) = line.strip_prefix("data: ").or_else(|| line.strip_prefix("data:"))
            else {
                continue;
            };
            if data.trim() == "[DONE]" {
                return Ok(events);
            }
            let chunk: ChatChunk = serde_json::from_str(data)?;
            events.extend(map_chunk(chunk, &mut tool_call_ids));
        }
    }
    Ok(events)
}
