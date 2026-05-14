//! Content-addressed binary blobs.
//!
//! `BlobHash` is the SHA-256 of the raw bytes of the blob (NOT of canonical
//! JSON — blobs are arbitrary bytes, not necessarily JSON). Identical bytes
//! collide; storage is idempotent on PK.
//!
//! The Blob-store slot lives at the bottom of the memory stack: episodic
//! records hold a `content_hash[]` pointing into blob storage, and
//! everything else (NC-doc, reflective) ultimately resolves back to blob
//! bytes for verbatim recall.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::Error;

/// 32-byte SHA-256 over the raw blob bytes. Distinct from `EventId` because
/// EventId is over *canonical JSON*, while BlobHash is over arbitrary bytes.
#[derive(Clone, Copy, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct BlobHash(#[serde(with = "hex_bytes")] pub [u8; 32]);

impl BlobHash {
    pub fn compute(bytes: &[u8]) -> Self {
        let digest = Sha256::digest(bytes);
        let mut out = [0u8; 32];
        out.copy_from_slice(&digest);
        BlobHash(out)
    }

    pub fn to_hex(self) -> String {
        hex::encode(self.0)
    }

    pub fn from_hex(s: &str) -> Result<Self, Error> {
        let bytes = hex::decode(s)?;
        if bytes.len() != 32 {
            return Err(Error::Signature(format!(
                "BlobHash must be 32 bytes, got {}",
                bytes.len()
            )));
        }
        let mut out = [0u8; 32];
        out.copy_from_slice(&bytes);
        Ok(BlobHash(out))
    }
}

impl fmt::Debug for BlobHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "BlobHash({})", self.to_hex())
    }
}

impl fmt::Display for BlobHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

impl FromStr for BlobHash {
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

/// A blob: opaque bytes plus a content_type hint. Hashing depends only on
/// `bytes`; `content_type` is a metadata hint, not part of the identity.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Blob {
    pub bytes: Vec<u8>,
    pub content_type: String,
}

impl Blob {
    pub fn new(bytes: Vec<u8>, content_type: impl Into<String>) -> Self {
        Self {
            bytes,
            content_type: content_type.into(),
        }
    }

    pub fn octet_stream(bytes: Vec<u8>) -> Self {
        Self::new(bytes, "application/octet-stream")
    }

    pub fn text(text: impl Into<String>) -> Self {
        let s = text.into();
        Self::new(s.into_bytes(), "text/plain; charset=utf-8")
    }

    pub fn hash(&self) -> BlobHash {
        BlobHash::compute(&self.bytes)
    }

    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_bytes_collide() {
        let a = Blob::text("hello");
        let b = Blob::text("hello");
        assert_eq!(a.hash(), b.hash());
    }

    #[test]
    fn distinct_bytes_distinct_hashes() {
        let a = Blob::text("hello");
        let b = Blob::text("world");
        assert_ne!(a.hash(), b.hash());
    }

    #[test]
    fn content_type_does_not_affect_hash() {
        // The hash is identity by bytes alone. Two blobs with the same bytes
        // and different content_types collide — that's the invariant that
        // lets the storage layer dedupe regardless of how the caller tags.
        let a = Blob::new(b"x".to_vec(), "application/json");
        let b = Blob::new(b"x".to_vec(), "text/plain");
        assert_eq!(a.hash(), b.hash());
    }

    #[test]
    fn hash_hex_is_64_chars() {
        assert_eq!(Blob::text("x").hash().to_hex().len(), 64);
    }

    #[test]
    fn hash_round_trips_via_hex() {
        let h = Blob::text("x").hash();
        assert_eq!(BlobHash::from_hex(&h.to_hex()).unwrap(), h);
    }

    #[test]
    fn matches_known_sha256() {
        // Cross-implementation sanity: sha256("hello") = 2cf24dba5fb0a30e...
        let h = Blob::text("hello").hash();
        assert_eq!(
            h.to_hex(),
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
    }
}
