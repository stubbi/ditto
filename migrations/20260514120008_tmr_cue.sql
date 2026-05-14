-- Targeted Memory Reactivation cues.
--
-- The architecture's `memory.tmr(focus, hint)` surface. A cue biases the
-- next dream cycle to prioritize events related to a specific topic —
-- "Rasch et al. 2007" — without changing the underlying retrieval model.
-- Persisted because cues set in one turn need to survive process restart
-- and be consumed by a later (possibly background) dream sweep.

CREATE TABLE tmr_cue (
    cue_id      uuid        PRIMARY KEY,
    tenant_id   uuid        NOT NULL,
    scope_id    uuid,
    focus       text        NOT NULL,
    hint        text,
    set_at      timestamptz NOT NULL DEFAULT now(),
    consumed_at timestamptz
);

-- Hot path: "pending cues for tenant X" — the dream cycle's opening
-- read on every run.
CREATE INDEX tmr_cue_tenant_pending_idx
    ON tmr_cue (tenant_id)
    WHERE consumed_at IS NULL;
