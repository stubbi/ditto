-- Ditto memory schema, v1 (episodic + receipt).
--
-- Bi-temporal nc_node / nc_edge, procedural index, and pgvector HNSW indices
-- follow in subsequent migrations. RLS policies follow once we have a
-- tenant-aware connection role.

-- The episodic-index slot: pointers and sparse keys, content lives in
-- blob storage (forthcoming). v0 inlines `payload` JSONB to bootstrap;
-- subsequent migration will split blob_store out.
CREATE TABLE episodic (
    event_id        bytea       PRIMARY KEY,        -- sha256(canonical_json(payload))
    prev_event_id   bytea       REFERENCES episodic(event_id),
    tenant_id       uuid        NOT NULL,
    scope_id        uuid        NOT NULL,
    source_id       text        NOT NULL,
    slot            text        NOT NULL,
    payload         jsonb       NOT NULL,
    content         text,                            -- denormalized payload->>'content' for tsvector
    content_tsv     tsvector    GENERATED ALWAYS AS (to_tsvector('simple', coalesce(content, ''))) STORED,
    ts              timestamptz NOT NULL,
    schema_version  integer     NOT NULL
);

CREATE INDEX episodic_tenant_ts_idx ON episodic (tenant_id, ts DESC);
CREATE INDEX episodic_tenant_source_ts_idx ON episodic (tenant_id, source_id, ts DESC);
CREATE INDEX episodic_content_tsv_idx ON episodic USING GIN (content_tsv);

-- SCITT-style signed receipts. Per-(tenant, source) hash chain via prev_event_id.
CREATE TABLE receipt (
    event_id        bytea       PRIMARY KEY REFERENCES episodic(event_id),
    prev_event_id   bytea,
    tenant_id       uuid        NOT NULL,
    source_id       text        NOT NULL,
    schema_version  integer     NOT NULL,
    signature       bytea,                           -- Ed25519 detached sig over event signing_bytes
    ts              timestamptz NOT NULL
);

CREATE INDEX receipt_tenant_ts_idx ON receipt (tenant_id, ts DESC);
