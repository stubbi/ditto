-- Reflective slot — consolidator-derived higher-order representations.
--
-- Bi-temporal columns mirror nc_edge: contradicting reflections invalidate
-- prior ones (set t_invalid) rather than overwriting. Audit of how the
-- agent's beliefs evolved is preserved by construction.
--
-- source_event_ids cites the episodic events the consolidator considered,
-- and consolidation_receipt cites the event_id of the consolidator's own
-- commit receipt — that's what lets us verify a reflection was produced by
-- a recognised consolidation pass rather than injected from outside.

CREATE TABLE reflective (
    reflective_id          uuid        PRIMARY KEY,
    tenant_id              uuid        NOT NULL,
    scope_id               uuid        NOT NULL,
    content                text        NOT NULL,
    confidence             real        NOT NULL DEFAULT 0.5,
    source_event_ids       bytea[]     NOT NULL DEFAULT ARRAY[]::bytea[],
    consolidation_receipt  bytea,
    t_created              timestamptz NOT NULL DEFAULT now(),
    t_expired              timestamptz,
    t_valid                timestamptz NOT NULL,
    t_invalid              timestamptz,
    CHECK (confidence >= 0.0 AND confidence <= 1.0)
);

CREATE INDEX reflective_tenant_t_valid_idx ON reflective (tenant_id, t_valid DESC);
CREATE INDEX reflective_tenant_scope_idx ON reflective (tenant_id, scope_id);
-- Partial index for the hot path: "current reflections for tenant X".
CREATE INDEX reflective_current_idx
    ON reflective (tenant_id, scope_id)
    WHERE t_expired IS NULL AND t_invalid IS NULL;
