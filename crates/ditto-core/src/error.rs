use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("invalid hex: {0}")]
    InvalidHex(#[from] hex::FromHexError),

    #[error("invalid uuid: {0}")]
    InvalidUuid(#[from] uuid::Error),

    #[error("canonical json error: {0}")]
    CanonicalJson(String),

    #[error("signature error: {0}")]
    Signature(String),

    #[error("serde_json: {0}")]
    Json(#[from] serde_json::Error),

    #[error("{0}")]
    Other(String),
}
