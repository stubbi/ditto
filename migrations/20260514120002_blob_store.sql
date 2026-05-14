-- Blob-store CAS slot.
--
-- Content-addressed binary storage. Episodic.content_hash[] (added in this
-- migration) points into here. Identical bytes collide on `content_hash`
-- (the SHA-256 of `payload`), making writes idempotent — the same blob
-- arriving from two different code paths costs one row, not two.
--
-- Tenancy: the table is partitioned by (tenant_id, content_hash) rather than
-- just content_hash, even though the hash is intrinsic. Same bytes for two
-- tenants are stored twice so a delete by one tenant cannot leak through the
-- isolation boundary. This is the same invariant InMemoryStorage enforces.
--
-- bytea PK on (tenant_id, content_hash) is acceptable up to ~10M blobs per
-- tenant. Beyond that, push payloads to S3-compatible object storage and
-- keep this table as the metadata index. Defer that split until measured.

CREATE TABLE blob (
    tenant_id       uuid        NOT NULL,
    content_hash    bytea       NOT NULL,
    content_type    text        NOT NULL DEFAULT 'application/octet-stream',
    payload         bytea       NOT NULL,
    bytelen         integer     NOT NULL,
    ts              timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (tenant_id, content_hash)
);

CREATE INDEX blob_tenant_ts_idx ON blob (tenant_id, ts DESC);

-- Episodic records can now reference blob hashes explicitly. Stays NULL-free
-- and empty by default so existing rows (v0 inlined payload) remain valid.
-- Migration order matters: the array column is additive; readers that don't
-- know about it ignore it; readers that do know about it dereference into
-- `blob`.
ALTER TABLE episodic
    ADD COLUMN content_hash bytea[] NOT NULL DEFAULT ARRAY[]::bytea[];
