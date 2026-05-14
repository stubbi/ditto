//! Identifier types: content-addressed `EventId`, plus tenant and scope UUIDs.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::canonical::to_canonical_bytes;
use crate::error::Error;

/// Content-addressed event identifier. Always the SHA-256 of the canonical JSON
/// encoding of the event payload. Identical payloads collide, making writes
/// idempotent.
#[derive(Clone, Copy, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct EventId(#[serde(with = "hex_bytes")] pub [u8; 32]);

impl EventId {
    /// Compute the event_id from a payload by canonical-encoding then hashing.
    pub fn from_payload(payload: &serde_json::Value) -> Result<Self, Error> {
        let bytes = to_canonical_bytes(payload)?;
        let mut hasher = Sha256::new();
        hasher.update(&bytes);
        let digest = hasher.finalize();
        let mut out = [0u8; 32];
        out.copy_from_slice(&digest);
        Ok(EventId(out))
    }

    pub fn to_hex(self) -> String {
        hex::encode(self.0)
    }

    pub fn from_hex(s: &str) -> Result<Self, Error> {
        let bytes = hex::decode(s)?;
        if bytes.len() != 32 {
            return Err(Error::Signature(format!(
                "EventId must be 32 bytes, got {}",
                bytes.len()
            )));
        }
        let mut out = [0u8; 32];
        out.copy_from_slice(&bytes);
        Ok(EventId(out))
    }
}

impl fmt::Debug for EventId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "EventId({})", self.to_hex())
    }
}

impl fmt::Display for EventId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

impl FromStr for EventId {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::from_hex(s)
    }
}

mod hex_bytes {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(bytes: &[u8; 32], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&hex::encode(bytes))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<[u8; 32], D::Error> {
        let s = String::deserialize(d)?;
        let bytes = hex::decode(&s).map_err(serde::de::Error::custom)?;
        if bytes.len() != 32 {
            return Err(serde::de::Error::custom(format!(
                "expected 32 bytes, got {}",
                bytes.len()
            )));
        }
        let mut out = [0u8; 32];
        out.copy_from_slice(&bytes);
        Ok(out)
    }
}

/// Tenant identifier. The isolation boundary.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct TenantId(pub Uuid);

impl TenantId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for TenantId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for TenantId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl FromStr for TenantId {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(Uuid::parse_str(s)?))
    }
}

/// Scope identifier. Workspace / matter / institutional / etc.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct ScopeId(pub Uuid);

impl ScopeId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for ScopeId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for ScopeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl FromStr for ScopeId {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(Uuid::parse_str(s)?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn event_id_is_deterministic() {
        let a = EventId::from_payload(&json!({"content": "hi", "ts": 1})).unwrap();
        let b = EventId::from_payload(&json!({"ts": 1, "content": "hi"})).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn event_id_hex_is_64_chars() {
        let id = EventId::from_payload(&json!({"x": 1})).unwrap();
        assert_eq!(id.to_hex().len(), 64);
    }

    #[test]
    fn event_id_roundtrip_via_hex() {
        let a = EventId::from_payload(&json!({"x": 1})).unwrap();
        let b = EventId::from_hex(&a.to_hex()).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn event_id_matches_python_fixture() {
        // Cross-language interop check. The Python eval harness computes
        // sha256(canonical_json({"content": "hi", "ts": 1})) via
        // `ditto_eval.types.content_address`. This hash was confirmed by
        // running the Python implementation directly. Do not edit this
        // constant without also updating the Python fixture — both
        // implementations must produce the same event_id for the same
        // payload, otherwise events written by one cannot be read by the
        // other.
        let id = EventId::from_payload(&json!({"content": "hi", "ts": 1})).unwrap();
        assert_eq!(
            id.to_hex(),
            "54574ccc73eb92a5475f436d116e68fc323d757993508f6eea50af875a72347e"
        );
    }
}
