use crate::auth::AuthHandle;
use crate::capabilities::CapabilitySet;
use crate::model::{Call, ModelDescriptor, ProviderId};
use crate::stream::EventStream;
use crate::Error;
use async_trait::async_trait;

pub mod openrouter;

/// The Provider contract.
///
/// `Capabilities` is an associated type rather than `Box<dyn CapabilitySet>` so
/// that capability projection through `ProviderExtensions` can be checked at
/// compile time when the provider is statically known. Erased usage (the
/// router selecting at runtime) goes through `ErasedProvider`.
#[async_trait]
pub trait Provider: Send + Sync + 'static {
    type Capabilities: CapabilitySet;

    fn id(&self) -> ProviderId;
    fn models(&self) -> &[ModelDescriptor];
    fn capabilities(&self) -> Self::Capabilities;

    async fn stream(&self, call: Call, auth: &AuthHandle) -> Result<EventStream, Error>;
}

/// Object-safe view of `Provider` for use in the router. Capability lookups go
/// through an erased trait object; statically known provider extensions are
/// only available via the concrete type.
#[async_trait]
pub trait ErasedProvider: Send + Sync + 'static {
    fn id(&self) -> ProviderId;
    fn models(&self) -> &[ModelDescriptor];
    fn capabilities(&self) -> &dyn CapabilitySet;
    async fn stream(&self, call: Call, auth: &AuthHandle) -> Result<EventStream, Error>;
}
