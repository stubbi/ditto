//! Core data types: `Event`, `Receipt`, `Slot`, `SchemaVersion`.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::error::Error;
use crate::id::{EventId, ScopeId, TenantId};
use crate::signing::{InstallKey, Signature, VerifyingKey};

/// Schema version for canonical encoding + receipt layout. Increment when the
/// shape of signed bytes changes.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SchemaVersion(pub u32);

impl SchemaVersion {
    pub fn as_u32(self) -> u32 {
        self.0
    }
}

/// Typed memory slot. The data model does not grow beyond this set.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Slot {
    /// In-context scratchpad. Never persisted.
    Working,
    /// Append-only event index (pointers + sparse keys, salience).
    EpisodicIndex,
    /// Content-addressed blob — raw transcripts, tool outputs, file diffs.
    BlobStore,
    /// Bi-temporal typed property graph (semantic facts).
    NcGraph,
    /// Compiled per-entity Markdown pages (filesystem-as-memory).
    NcDoc,
    /// Skills with lifecycle (active / deprecated / archived).
    Procedural,
    /// Consolidator-derived reflections.
    Reflective,
}

/// A unit written to memory.
///
/// `event_id` is the SHA-256 of the canonical JSON of `payload`. Identical
/// payloads produce identical event_ids; the storage layer dedupes on PK.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Event {
    pub event_id: EventId,
    pub prev_event_id: Option<EventId>,
    pub tenant_id: TenantId,
    pub scope_id: ScopeId,
    pub source_id: String,
    pub slot: Slot,
    pub payload: serde_json::Value,
    pub timestamp: DateTime<Utc>,
}

impl Event {
    /// Build an event, computing `event_id` from `payload`.
    pub fn new(
        tenant_id: TenantId,
        scope_id: ScopeId,
        source_id: impl Into<String>,
        slot: Slot,
        payload: serde_json::Value,
        timestamp: DateTime<Utc>,
        prev_event_id: Option<EventId>,
    ) -> Result<Self, Error> {
        let event_id = EventId::from_payload(&payload)?;
        Ok(Self {
            event_id,
            prev_event_id,
            tenant_id,
            scope_id,
            source_id: source_id.into(),
            slot,
            payload,
            timestamp,
        })
    }

    /// The bytes signed in a Receipt for this event.
    ///
    /// Format (v1): `event_id || prev_event_id (or 32 zero bytes) || tenant_id ||
    /// source_id_canonical_json || timestamp_unix_nanos_be || schema_version_be`.
    /// Deterministic; recomputable by any verifier given the receipt header.
    pub fn signing_bytes(&self, schema_version: SchemaVersion) -> Vec<u8> {
        let mut buf = Vec::with_capacity(128);
        buf.extend_from_slice(&self.event_id.0);
        match self.prev_event_id {
            Some(p) => buf.extend_from_slice(&p.0),
            None => buf.extend_from_slice(&[0u8; 32]),
        }
        buf.extend_from_slice(self.tenant_id.0.as_bytes());
        // length-prefixed source_id so concat is unambiguous
        let src = self.source_id.as_bytes();
        buf.extend_from_slice(&(src.len() as u32).to_be_bytes());
        buf.extend_from_slice(src);
        let nanos = self
            .timestamp
            .timestamp_nanos_opt()
            .unwrap_or_else(|| self.timestamp.timestamp() * 1_000_000_000);
        buf.extend_from_slice(&nanos.to_be_bytes());
        buf.extend_from_slice(&schema_version.0.to_be_bytes());
        buf
    }
}

/// A signed acknowledgement that an event was committed.
///
/// `signature` is `None` only in test/embedded modes that explicitly disable
/// signing. Production deployments always sign.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Receipt {
    pub event_id: EventId,
    pub prev_event_id: Option<EventId>,
    pub tenant_id: TenantId,
    pub source_id: String,
    pub timestamp: DateTime<Utc>,
    pub schema_version: SchemaVersion,
    pub signature: Option<Signature>,
}

impl Receipt {
    /// Build a signed receipt for `event` using the install's key.
    pub fn sign(event: &Event, schema_version: SchemaVersion, key: &InstallKey) -> Self {
        let bytes = event.signing_bytes(schema_version);
        let signature = Some(key.sign(&bytes));
        Self {
            event_id: event.event_id,
            prev_event_id: event.prev_event_id,
            tenant_id: event.tenant_id,
            source_id: event.source_id.clone(),
            timestamp: event.timestamp,
            schema_version,
            signature,
        }
    }

    /// Build an unsigned receipt — only valid in test/embedded modes.
    pub fn unsigned(event: &Event, schema_version: SchemaVersion) -> Self {
        Self {
            event_id: event.event_id,
            prev_event_id: event.prev_event_id,
            tenant_id: event.tenant_id,
            source_id: event.source_id.clone(),
            timestamp: event.timestamp,
            schema_version,
            signature: None,
        }
    }

    /// Verify this receipt against the verifying key and the original event.
    pub fn verify(&self, event: &Event, verifier: &VerifyingKey) -> Result<(), Error> {
        let sig = self
            .signature
            .as_ref()
            .ok_or_else(|| Error::Signature("receipt has no signature".into()))?;
        if self.event_id != event.event_id {
            return Err(Error::Signature(
                "receipt event_id does not match event".into(),
            ));
        }
        let bytes = event.signing_bytes(self.schema_version);
        verifier.verify(&bytes, sig)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn make_event() -> Event {
        Event::new(
            TenantId::new(),
            ScopeId::new(),
            "test-source",
            Slot::EpisodicIndex,
            json!({"content": "hello"}),
            "2026-05-14T12:00:00Z".parse().unwrap(),
            None,
        )
        .unwrap()
    }

    #[test]
    fn event_id_matches_payload() {
        let e = make_event();
        let recomputed = EventId::from_payload(&e.payload).unwrap();
        assert_eq!(e.event_id, recomputed);
    }

    #[test]
    fn signed_receipt_verifies() {
        let key = InstallKey::generate();
        let event = make_event();
        let receipt = Receipt::sign(&event, SchemaVersion(1), &key);
        receipt.verify(&event, &key.verifying_key()).unwrap();
    }

    #[test]
    fn unsigned_receipt_fails_verify() {
        let key = InstallKey::generate();
        let event = make_event();
        let receipt = Receipt::unsigned(&event, SchemaVersion(1));
        assert!(receipt.verify(&event, &key.verifying_key()).is_err());
    }

    #[test]
    fn tampered_event_fails_verify() {
        let key = InstallKey::generate();
        let mut event = make_event();
        let receipt = Receipt::sign(&event, SchemaVersion(1), &key);
        event.source_id = "different-source".into();
        assert!(receipt.verify(&event, &key.verifying_key()).is_err());
    }

    #[test]
    fn signing_bytes_are_deterministic() {
        let event = make_event();
        let a = event.signing_bytes(SchemaVersion(1));
        let b = event.signing_bytes(SchemaVersion(1));
        assert_eq!(a, b);
    }

    #[test]
    fn signing_bytes_change_with_schema_version() {
        let event = make_event();
        let a = event.signing_bytes(SchemaVersion(1));
        let b = event.signing_bytes(SchemaVersion(2));
        assert_ne!(a, b);
    }
}
