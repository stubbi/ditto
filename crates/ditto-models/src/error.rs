use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("provider not registered: {0}")]
    UnknownProvider(String),

    #[error("model not in catalog: {0}")]
    UnknownModel(String),

    #[error("capability not supported by provider: {0}")]
    UnsupportedCapability(&'static str),

    #[error("authentication not available: {0}")]
    AuthUnavailable(String),

    #[error("subscription policy blocks this backend: {0}")]
    PolicyBlocked(String),

    #[error("schema encoding: {0}")]
    Schema(#[from] ditto_core::Error),

    #[error("serialization: {0}")]
    Serde(#[from] serde_json::Error),

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}
