-- Procedural slot (skills) — metadata index.
--
-- Skill *content* lives on the filesystem; this table is the index. A skill
-- is `(tenant_id, skill_id)` keyed — globally-unique skill IDs (as the
-- architecture doc originally sketched) would force every deployment to
-- coordinate on a namespace and add no operational benefit. The composite PK
-- here matches the blob-store tenancy pattern.
--
-- `last_used` and `tests_pass` are what the dream-cycle metabolism rules
-- read to decide deprecation (last_used > 30d OR tests_pass < 0.7 in the
-- v2 spec). v0 ships the columns + status transitions; the GC daemon that
-- consumes them lands with the consolidator.

CREATE TABLE procedural (
    tenant_id       uuid        NOT NULL,
    skill_id        text        NOT NULL,
    scope_id        uuid        NOT NULL,
    version         text        NOT NULL,
    path            text        NOT NULL,
    last_used       timestamptz,
    tests_pass      real,
    status          text        NOT NULL,
    PRIMARY KEY (tenant_id, skill_id),
    CHECK (status IN ('active', 'deprecated', 'archived')),
    CHECK (tests_pass IS NULL OR (tests_pass >= 0.0 AND tests_pass <= 1.0))
);

CREATE INDEX procedural_tenant_status_idx ON procedural (tenant_id, status);
CREATE INDEX procedural_tenant_last_used_idx ON procedural (tenant_id, last_used);
