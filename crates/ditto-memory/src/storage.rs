//! The `Storage` trait. Implementations: `InMemoryStorage` (this crate) and
//! `PostgresStorage` (in `ditto-storage-postgres`).
//!
//! Storage is the durable side of the controller. It must be linearizable on
//! `write` per `(tenant_id, source_id)` — the controller does not provide that
//! guarantee on its own. In practice this means: write inside a transaction,
//! take an advisory lock or unique-on-event_id, and let the database reject
//! conflicts.

use async_trait::async_trait;

use ditto_core::{Event, EventId, Receipt, TenantId};

use crate::search::{SearchQuery, SearchResult};

#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("storage error: {0}")]
    Other(String),

    #[error("event not found: {0}")]
    NotFound(EventId),

    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("ditto-core: {0}")]
    Core(#[from] ditto_core::Error),
}

pub type StorageResult<T> = Result<T, StorageError>;

#[async_trait]
pub trait Storage: Send + Sync {
    /// Commit an event and its signed receipt atomically.
    ///
    /// MUST be idempotent on `event.event_id`: a second write of the same
    /// event_id returns the existing receipt without modifying state.
    async fn commit(&self, event: &Event, receipt: &Receipt) -> StorageResult<()>;

    /// Fetch the receipt for an event_id, if present.
    async fn get_receipt(&self, event_id: &EventId) -> StorageResult<Option<Receipt>>;

    /// Fetch the original event (for receipt verification).
    async fn get_event(&self, event_id: &EventId) -> StorageResult<Option<Event>>;

    /// Search the tenant's memory.
    async fn search(&self, query: &SearchQuery) -> StorageResult<Vec<SearchResult>>;

    /// Wipe all data for `tenant_id`. Used between benchmark runs.
    async fn reset(&self, tenant_id: TenantId) -> StorageResult<()>;
}
