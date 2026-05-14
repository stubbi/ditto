//! Reference in-memory `Storage` implementation.
//!
//! Not a production backend. Used for:
//! - Unit testing the controller without a database
//! - The Python eval harness's `stub` backend equivalent — control floor for
//!   substring scanning
//! - A placeholder for the future SQLite embedded mode (same shape, durable
//!   substrate)
//!
//! Retrieval is naive substring matching with a recency tiebreak. The point is
//! to validate the orchestration, not the retrieval quality.

use std::collections::HashMap;
use std::sync::Mutex;

use async_trait::async_trait;
use serde_json::json;

use ditto_core::{Event, EventId, Receipt, TenantId};

use crate::search::{SearchQuery, SearchResult};
use crate::storage::{Storage, StorageResult};

#[derive(Default)]
pub struct InMemoryStorage {
    inner: Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
    /// tenant_id -> append-only list of events
    events: HashMap<TenantId, Vec<Event>>,
    /// event_id -> receipt
    receipts: HashMap<EventId, Receipt>,
}

impl InMemoryStorage {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl Storage for InMemoryStorage {
    async fn commit(&self, event: &Event, receipt: &Receipt) -> StorageResult<()> {
        let mut inner = self.inner.lock().unwrap();
        // idempotent on event_id
        if inner.receipts.contains_key(&event.event_id) {
            return Ok(());
        }
        inner
            .events
            .entry(event.tenant_id)
            .or_default()
            .push(event.clone());
        inner.receipts.insert(event.event_id, receipt.clone());
        Ok(())
    }

    async fn get_receipt(&self, event_id: &EventId) -> StorageResult<Option<Receipt>> {
        Ok(self.inner.lock().unwrap().receipts.get(event_id).cloned())
    }

    async fn get_event(&self, event_id: &EventId) -> StorageResult<Option<Event>> {
        let inner = self.inner.lock().unwrap();
        for events in inner.events.values() {
            if let Some(e) = events.iter().find(|e| &e.event_id == event_id) {
                return Ok(Some(e.clone()));
            }
        }
        Ok(None)
    }

    async fn search(&self, query: &SearchQuery) -> StorageResult<Vec<SearchResult>> {
        let inner = self.inner.lock().unwrap();
        let events = match inner.events.get(&query.tenant_id) {
            Some(v) => v,
            None => return Ok(Vec::new()),
        };
        let q = query.query.to_lowercase();
        let mut scored: Vec<(f32, &Event)> = Vec::new();
        for e in events {
            if let Some(sources) = &query.sources {
                if !sources.contains(&e.source_id) {
                    continue;
                }
            }
            if let Some(scope_id) = query.scope_id {
                if e.scope_id != scope_id {
                    continue;
                }
            }
            let content = render(e);
            let hits = content.to_lowercase().matches(&q).count();
            if hits == 0 {
                continue;
            }
            let recency_tiebreak = e.timestamp.timestamp() as f32 / 1e12;
            scored.push((hits as f32 + recency_tiebreak, e));
        }
        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        let out = scored
            .into_iter()
            .take(query.k)
            .map(|(score, e)| SearchResult {
                event_id: e.event_id,
                content: render(e),
                score,
                source_event_ids: vec![e.event_id],
                metadata: json!({
                    "source_id": e.source_id,
                    "timestamp": e.timestamp,
                    "slot": e.slot,
                }),
            })
            .collect();
        Ok(out)
    }

    async fn reset(&self, tenant_id: TenantId) -> StorageResult<()> {
        let mut inner = self.inner.lock().unwrap();
        if let Some(events) = inner.events.remove(&tenant_id) {
            for e in events {
                inner.receipts.remove(&e.event_id);
            }
        }
        Ok(())
    }
}

fn render(event: &Event) -> String {
    if let Some(s) = event.payload.get("content").and_then(|v| v.as_str()) {
        s.to_string()
    } else {
        event.payload.to_string()
    }
}
