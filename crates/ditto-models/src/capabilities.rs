use serde::{Deserialize, Serialize};

/// Typed capability projection.
///
/// A provider that lies about a capability silently is the LiteLLM-#14293 /
/// AI-SDK-`providerOptions`-opacity failure mode. By forcing every capability
/// through an enum, a `Call` that asks for cache-control against a provider
/// whose `prompt_caching() == Implicit` errors at build-request time instead
/// of being silently stripped.
pub trait CapabilitySet: Send + Sync {
    fn tool_calling(&self) -> ToolCallingShape;
    fn prompt_caching(&self) -> CachingMode;
    fn reasoning(&self) -> ReasoningMode;
    fn multimodal(&self) -> MultimodalCaps;
    fn batching(&self) -> bool;
    fn native_web_search(&self) -> bool;
    fn rate_limit_headers(&self) -> RateLimitHeaderShape;
    fn structured_outputs(&self) -> StructuredOutputMode;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ToolCallingShape {
    None,
    OpenAi,
    Anthropic,
    Gemini,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum CachingMode {
    None,
    /// Provider caches automatically; no per-call hint surface.
    Implicit,
    /// Caller writes `cache_control` markers per content block (Anthropic).
    ExplicitCacheControl,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ReasoningMode {
    None,
    /// Provider bills reasoning tokens but does not expose the trace
    /// (OpenAI o1, Gemini Thinking with `include_thoughts = false`).
    Hidden,
    /// Provider emits typed reasoning blocks (Anthropic extended thinking,
    /// DeepSeek `<think>` tags after our normalization).
    ExplicitBlocks,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct MultimodalCaps {
    pub image_input: bool,
    pub audio_input: bool,
    pub video_input: bool,
    pub pdf_input: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum RateLimitHeaderShape {
    None,
    OpenAi,
    Anthropic,
    Bedrock,
    Vertex,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum StructuredOutputMode {
    None,
    JsonMode,
    StrictSchema,
}
