-- Persist the reconsolidation labile window and the shadow chain.
--
-- In v0 these lived in `Mutex<HashMap>` on the controller, which meant a
-- process restart lost both the "this event is still rewriteable" state and
-- the audit trail of which events shadow which. That works for tests but
-- fails the architectural commitment: the reconsolidation mechanism is the
-- prompt-injection mitigation, and it must survive crashes.

-- A row exists for every event whose labile window is currently open. The
-- background pruner sweeps rows where labile_until < now() periodically.
CREATE TABLE labile_window (
    tenant_id     uuid        NOT NULL,
    event_id      bytea       NOT NULL,
    labile_until  timestamptz NOT NULL,
    opened_at     timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (tenant_id, event_id)
);
CREATE INDEX labile_window_until_idx ON labile_window (labile_until);

-- One row per "original event was shadowed by another". A shadow chain
-- A→B→C means lookup(A) returns B, lookup(B) returns C; the bounded walker
-- in the controller terminates at C (max 8 hops) or earlier if a cycle is
-- detected.
CREATE TABLE event_shadow (
    tenant_id  uuid        NOT NULL,
    original   bytea       NOT NULL,
    shadow     bytea       NOT NULL,
    authority  text        NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (tenant_id, original),
    CHECK (authority IN ('user', 'verified_tool', 'system_admin'))
);
CREATE INDEX event_shadow_tenant_idx ON event_shadow (tenant_id);
