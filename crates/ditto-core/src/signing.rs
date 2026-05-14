//! Ed25519 signing for memory receipts.
//!
//! Each Ditto install holds one signing key. Every receipt carries a detached
//! Ed25519 signature over the canonical bytes of the receipt header (event_id,
//! prev_event_id, tenant_id, source_id, timestamp, schema_version). This
//! enables offline receipt verification against the install's public key and
//! forms the basis of the SCITT-compliant Merkle log built one layer up.

use ed25519_dalek::{Signer, SigningKey, Verifier, VerifyingKey as DalekVerifyingKey};
use serde::{Deserialize, Serialize};

use crate::error::Error;

/// Detached Ed25519 signature, hex-encoded for JSON transport.
#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct Signature(#[serde(with = "hex_sig")] pub [u8; 64]);

impl Signature {
    pub fn to_hex(&self) -> String {
        hex::encode(self.0)
    }

    pub fn from_hex(s: &str) -> Result<Self, Error> {
        let bytes = hex::decode(s)?;
        if bytes.len() != 64 {
            return Err(Error::Signature(format!(
                "Signature must be 64 bytes, got {}",
                bytes.len()
            )));
        }
        let mut out = [0u8; 64];
        out.copy_from_slice(&bytes);
        Ok(Signature(out))
    }
}

impl std::fmt::Debug for Signature {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Signature({})", self.to_hex())
    }
}

mod hex_sig {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(bytes: &[u8; 64], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&hex::encode(bytes))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<[u8; 64], D::Error> {
        let s = String::deserialize(d)?;
        let bytes = hex::decode(&s).map_err(serde::de::Error::custom)?;
        if bytes.len() != 64 {
            return Err(serde::de::Error::custom(format!(
                "expected 64 bytes, got {}",
                bytes.len()
            )));
        }
        let mut out = [0u8; 64];
        out.copy_from_slice(&bytes);
        Ok(out)
    }
}

/// An install's Ed25519 signing key. Private; never serialized.
pub struct InstallKey {
    signing: SigningKey,
}

impl InstallKey {
    /// Generate a fresh signing key from the OS RNG.
    pub fn generate() -> Self {
        let mut csprng = rand::rngs::OsRng;
        Self {
            signing: SigningKey::generate(&mut csprng),
        }
    }

    /// Reconstruct from a 32-byte secret. Useful for tests + persisted keys.
    pub fn from_secret_bytes(bytes: &[u8]) -> Result<Self, Error> {
        if bytes.len() != 32 {
            return Err(Error::Signature(format!(
                "install key must be 32 bytes, got {}",
                bytes.len()
            )));
        }
        let mut buf = [0u8; 32];
        buf.copy_from_slice(bytes);
        Ok(Self {
            signing: SigningKey::from_bytes(&buf),
        })
    }

    pub fn secret_bytes(&self) -> [u8; 32] {
        self.signing.to_bytes()
    }

    pub fn verifying_key(&self) -> VerifyingKey {
        VerifyingKey(self.signing.verifying_key())
    }

    pub fn sign(&self, message: &[u8]) -> Signature {
        Signature(self.signing.sign(message).to_bytes())
    }
}

impl std::fmt::Debug for InstallKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InstallKey").finish_non_exhaustive()
    }
}

/// Public half of an install key. Distributable; used for receipt verification.
#[derive(Clone)]
pub struct VerifyingKey(DalekVerifyingKey);

impl VerifyingKey {
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, Error> {
        if bytes.len() != 32 {
            return Err(Error::Signature(format!(
                "verifying key must be 32 bytes, got {}",
                bytes.len()
            )));
        }
        let mut buf = [0u8; 32];
        buf.copy_from_slice(bytes);
        DalekVerifyingKey::from_bytes(&buf)
            .map(VerifyingKey)
            .map_err(|e| Error::Signature(format!("invalid verifying key: {e}")))
    }

    pub fn to_bytes(&self) -> [u8; 32] {
        self.0.to_bytes()
    }

    pub fn verify(&self, message: &[u8], sig: &Signature) -> Result<(), Error> {
        let dalek_sig = ed25519_dalek::Signature::from_bytes(&sig.0);
        self.0
            .verify(message, &dalek_sig)
            .map_err(|e| Error::Signature(format!("verification failed: {e}")))
    }
}

impl std::fmt::Debug for VerifyingKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "VerifyingKey({})", hex::encode(self.to_bytes()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sign_and_verify_roundtrip() {
        let key = InstallKey::generate();
        let msg = b"hello ditto";
        let sig = key.sign(msg);
        key.verifying_key().verify(msg, &sig).unwrap();
    }

    #[test]
    fn verification_fails_on_tampering() {
        let key = InstallKey::generate();
        let sig = key.sign(b"original");
        assert!(key.verifying_key().verify(b"tampered", &sig).is_err());
    }

    #[test]
    fn install_key_roundtrips_via_secret_bytes() {
        let k1 = InstallKey::generate();
        let k2 = InstallKey::from_secret_bytes(&k1.secret_bytes()).unwrap();
        let msg = b"x";
        let sig = k1.sign(msg);
        k2.verifying_key().verify(msg, &sig).unwrap();
    }

    #[test]
    fn signature_hex_roundtrips() {
        let key = InstallKey::generate();
        let sig = key.sign(b"x");
        let restored = Signature::from_hex(&sig.to_hex()).unwrap();
        assert_eq!(sig, restored);
    }
}
