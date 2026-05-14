# ditto-core

Core types for the Ditto agent memory system. No I/O dependencies.

## What's here

- **Canonical JSON encoding** (`canonical.rs`) — sorted keys, no whitespace, UTF-8. Cross-language compatible with `ditto_eval.types._canonical_json` in the Python eval harness. Both implementations produce identical bytes for identical payloads, so event_ids are interoperable.
- **Content addressing** (`id.rs`) — `EventId` is the SHA-256 of canonical JSON of an event payload. `TenantId` and `ScopeId` are UUIDs.
- **Ed25519 signing** (`signing.rs`) — `InstallKey` for signing receipts, `VerifyingKey` for offline verification. v0 of the SCITT-style receipt path.
- **Core data types** (`types.rs`) — `Event`, `Receipt`, `Slot`, `SchemaVersion`. Receipt signing bytes are deterministic and recomputable by any verifier given the receipt header.

## Cross-language interop

The Rust `EventId::from_payload(&payload)` and the Python `ditto_eval.types.content_address(payload)` produce identical 256-bit digests. Test `event_id_matches_python_fixture` enforces this with a checked-in fixture.

Do not change the canonical encoding or the receipt-signing-bytes layout without bumping `CURRENT_SCHEMA_VERSION`. Every persisted memory depends on these contracts staying stable.
