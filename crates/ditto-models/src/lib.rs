//! Ditto's model-routing layer.
//!
//! v0 scope: typed trait surface and data ontology. No provider HTTP impls
//! ship in this crate yet — each provider lands as its own research-first
//! commit per `docs/research/models/landscape.md`.
//!
//! The point of landing trait surface first (mirroring `ditto-eval`'s
//! `MemoryBackend` protocol) is to lock the contract before implementing it.
//! Adding a field to `Event` or a method to `Provider` later means rewriting
//! every adapter — so we want the shape committed and reviewed standalone.

pub mod auth;
pub mod capabilities;
pub mod cost;
pub mod error;
pub mod model;
pub mod provider;
pub mod stream;
pub mod tools;

pub use auth::{
    AccessToken, AuthHandle, LoginKind, LoginOutcome, PolicyStatus, RateLimitClass,
    SubscriptionBackend,
};
pub use capabilities::{
    CachingMode, CapabilitySet, MultimodalCaps, RateLimitHeaderShape, ReasoningMode,
    StructuredOutputMode, ToolCallingShape,
};
pub use cost::{CallCost, CostBreakdown};
pub use error::Error;
pub use model::{Call, ModelDescriptor, ModelRef, ProviderId, Region};
pub use provider::Provider;
pub use stream::{ErrorClass, Event, EventStream, FinishReason, ToolResultContent, Usage};
pub use tools::{
    ProjectionMode, Projected, SchemaHash, Tool, ToolId, ToolKind, ToolRegistry, TurnProjection,
};
