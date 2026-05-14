use thiserror::Error;

#[derive(Debug, Error)]
pub enum RenderError {
    #[error("storage error: {0}")]
    Storage(#[from] ditto_memory::storage::StorageError),

    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("manifest error: {0}")]
    Manifest(String),

    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
}
