//! The `MemoryController` — the single-writer commit path.
//!
//! v0 controller responsibilities:
//! - Compute the event_id from the payload (content addressing)
//! - Mint a signed receipt
//! - Hand off to storage in a single transaction
//! - Track the per-(tenant, source) hash-chain head for `prev_event_id`
//!
//! Deferred to follow-up commits:
//! - Surprise-gated writes (encoder prediction-error scoring)
//! - Reconsolidation labile window on retrieval
//! - Metacognitive retrieval gate (RSCB-MC contextual bandit)
//! - Awake-ripple / dream-cycle / long-sleep consolidation
//! - RL-trained operations policy (Memory-R1 / Mem-α lineage)

use std::collections::HashMap;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde_json::Value;
use tokio::sync::Mutex;

use ditto_core::{
    Event, EventId, InstallKey, Receipt, ScopeId, SchemaVersion, Slot, TenantId,
    CURRENT_SCHEMA_VERSION,
};

use crate::search::{SearchQuery, SearchResult};
use crate::storage::{Storage, StorageError, StorageResult};

#[derive(Clone, Copy)]
pub enum SigningPolicy {
    /// Sign every receipt. Production default.
    Required,
    /// Skip signing. Test/embedded only.
    Skip,
}

pub struct MemoryController<S: Storage> {
    storage: Arc<S>,
    install_key: Arc<InstallKey>,
    signing: SigningPolicy,
    schema_version: SchemaVersion,
    chain_heads: Mutex<HashMap<(TenantId, String), EventId>>,
}

impl<S: Storage> MemoryController<S> {
    pub fn new(storage: S, install_key: InstallKey) -> Self {
        Self {
            storage: Arc::new(storage),
            install_key: Arc::new(install_key),
            signing: SigningPolicy::Required,
            schema_version: CURRENT_SCHEMA_VERSION,
            chain_heads: Mutex::new(HashMap::new()),
        }
    }

    pub fn with_signing_policy(mut self, policy: SigningPolicy) -> Self {
        self.signing = policy;
        self
    }

    pub fn storage(&self) -> &S {
        &self.storage
    }

    /// Commit a payload as an event in `slot`. The event_id is computed from
    /// the payload (content addressing); idempotent on collision.
    pub async fn write(
        &self,
        tenant_id: TenantId,
        scope_id: ScopeId,
        source_id: impl Into<String>,
        slot: Slot,
        payload: Value,
        timestamp: DateTime<Utc>,
    ) -> StorageResult<Receipt> {
        let source_id = source_id.into();
        let prev = {
            let heads = self.chain_heads.lock().await;
            heads.get(&(tenant_id, source_id.clone())).copied()
        };
        let event = Event::new(tenant_id, scope_id, &source_id, slot, payload, timestamp, prev)
            .map_err(StorageError::from)?;

        // idempotency: if already committed, return existing receipt
        if let Some(receipt) = self.storage.get_receipt(&event.event_id).await? {
            return Ok(receipt);
        }

        let receipt = match self.signing {
            SigningPolicy::Required => Receipt::sign(&event, self.schema_version, &self.install_key),
            SigningPolicy::Skip => Receipt::unsigned(&event, self.schema_version),
        };

        self.storage.commit(&event, &receipt).await?;

        {
            let mut heads = self.chain_heads.lock().await;
            heads.insert((tenant_id, source_id), event.event_id);
        }

        Ok(receipt)
    }

    /// Verify a receipt against its event in storage.
    pub async fn verify(&self, receipt: &Receipt) -> StorageResult<bool> {
        let event = match self.storage.get_event(&receipt.event_id).await? {
            Some(e) => e,
            None => return Ok(false),
        };
        let verifier = self.install_key.verifying_key();
        Ok(receipt.verify(&event, &verifier).is_ok())
    }

    pub async fn search(&self, query: &SearchQuery) -> StorageResult<Vec<SearchResult>> {
        self.storage.search(query).await
    }

    pub async fn reset(&self, tenant_id: TenantId) -> StorageResult<()> {
        self.storage.reset(tenant_id).await?;
        let mut heads = self.chain_heads.lock().await;
        heads.retain(|(t, _), _| *t != tenant_id);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::in_memory::InMemoryStorage;
    use serde_json::json;

    fn now() -> DateTime<Utc> {
        Utc::now()
    }

    #[tokio::test]
    async fn write_returns_signed_receipt_that_verifies() {
        let ctrl = MemoryController::new(InMemoryStorage::new(), InstallKey::generate());
        let tenant = TenantId::new();
        let scope = ScopeId::new();
        let receipt = ctrl
            .write(
                tenant,
                scope,
                "test",
                Slot::EpisodicIndex,
                json!({"content": "hello"}),
                now(),
            )
            .await
            .unwrap();
        assert!(receipt.signature.is_some());
        assert!(ctrl.verify(&receipt).await.unwrap());
    }

    #[tokio::test]
    async fn write_is_idempotent_on_event_id() {
        let ctrl = MemoryController::new(InMemoryStorage::new(), InstallKey::generate());
        let tenant = TenantId::new();
        let scope = ScopeId::new();
        let ts = now();
        let payload = json!({"content": "hi"});
        let r1 = ctrl
            .write(tenant, scope, "s", Slot::EpisodicIndex, payload.clone(), ts)
            .await
            .unwrap();
        let r2 = ctrl
            .write(tenant, scope, "s", Slot::EpisodicIndex, payload, ts)
            .await
            .unwrap();
        assert_eq!(r1.event_id, r2.event_id);
    }

    #[tokio::test]
    async fn hash_chain_advances_within_source() {
        let ctrl = MemoryController::new(InMemoryStorage::new(), InstallKey::generate());
        let tenant = TenantId::new();
        let scope = ScopeId::new();
        let r1 = ctrl
            .write(
                tenant,
                scope,
                "src-A",
                Slot::EpisodicIndex,
                json!({"content": "first"}),
                now(),
            )
            .await
            .unwrap();
        let r2 = ctrl
            .write(
                tenant,
                scope,
                "src-A",
                Slot::EpisodicIndex,
                json!({"content": "second"}),
                now(),
            )
            .await
            .unwrap();
        assert_eq!(r2.prev_event_id, Some(r1.event_id));
        assert_ne!(r1.event_id, r2.event_id);
    }

    #[tokio::test]
    async fn hash_chains_are_per_source() {
        let ctrl = MemoryController::new(InMemoryStorage::new(), InstallKey::generate());
        let tenant = TenantId::new();
        let scope = ScopeId::new();
        let _ = ctrl
            .write(
                tenant,
                scope,
                "src-A",
                Slot::EpisodicIndex,
                json!({"content": "a"}),
                now(),
            )
            .await
            .unwrap();
        let rb = ctrl
            .write(
                tenant,
                scope,
                "src-B",
                Slot::EpisodicIndex,
                json!({"content": "b"}),
                now(),
            )
            .await
            .unwrap();
        // First write in a new source has no prev_event_id
        assert_eq!(rb.prev_event_id, None);
    }

    #[tokio::test]
    async fn reset_clears_tenant_and_chain_heads() {
        let ctrl = MemoryController::new(InMemoryStorage::new(), InstallKey::generate());
        let tenant = TenantId::new();
        let scope = ScopeId::new();
        let _ = ctrl
            .write(
                tenant,
                scope,
                "s",
                Slot::EpisodicIndex,
                json!({"content": "x"}),
                now(),
            )
            .await
            .unwrap();
        ctrl.reset(tenant).await.unwrap();
        // After reset, next write has no prev_event_id
        let r = ctrl
            .write(
                tenant,
                scope,
                "s",
                Slot::EpisodicIndex,
                json!({"content": "y"}),
                now(),
            )
            .await
            .unwrap();
        assert_eq!(r.prev_event_id, None);
    }

    #[tokio::test]
    async fn search_finds_substring_match_on_in_memory_backend() {
        let ctrl = MemoryController::new(InMemoryStorage::new(), InstallKey::generate());
        let tenant = TenantId::new();
        let scope = ScopeId::new();
        ctrl.write(
            tenant,
            scope,
            "s",
            Slot::EpisodicIndex,
            json!({"content": "User said their birthday is March 14"}),
            now(),
        )
        .await
        .unwrap();
        ctrl.write(
            tenant,
            scope,
            "s",
            Slot::EpisodicIndex,
            json!({"content": "User asked about Berlin restaurants"}),
            now(),
        )
        .await
        .unwrap();
        let q = SearchQuery::new("birthday", tenant);
        let results = ctrl.search(&q).await.unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].content.contains("birthday"));
    }
}
