-- NC-graph: bi-temporal typed property graph.
--
-- v0 simplification: nodes are immutable. Nodes are stable concept handles
-- (Person, Project, Bug, Decision). All bi-temporal semantics live on edges.
-- Node-versioning (split into row_id PK + node_id logical identity) can be
-- introduced via a future migration without breaking edge queries.
--
-- Bi-temporal columns on edges:
--   t_created  — transaction time, when the system inserted this row
--   t_expired  — transaction time, when the system superseded this row
--                (NULL means this row is the latest transaction-time version)
--   t_valid    — valid time start, when the fact actually started being true
--   t_invalid  — valid time end, when the fact stopped being true
--                (NULL means no known end of validity)
--
-- An edge is "current" iff t_expired IS NULL AND t_invalid IS NULL.
-- An edge is "current as of T" (valid time) iff
--     t_valid <= T AND (t_invalid IS NULL OR t_invalid > T)
--     AND (t_expired IS NULL OR t_expired > T_tx)
-- where T_tx is the transaction-time at which the query is asked.

CREATE TABLE nc_node (
    node_id         uuid        PRIMARY KEY,
    tenant_id       uuid        NOT NULL,
    scope_id        uuid        NOT NULL,
    node_type       text        NOT NULL,
    properties      jsonb       NOT NULL DEFAULT '{}'::jsonb,
    t_created       timestamptz NOT NULL DEFAULT now(),
    provenance      bytea[]     NOT NULL DEFAULT ARRAY[]::bytea[]
);

CREATE INDEX nc_node_tenant_type_idx ON nc_node (tenant_id, node_type);
CREATE INDEX nc_node_tenant_scope_idx ON nc_node (tenant_id, scope_id);

CREATE TABLE nc_edge (
    edge_id         uuid        PRIMARY KEY,
    src             uuid        NOT NULL REFERENCES nc_node(node_id),
    dst             uuid        NOT NULL REFERENCES nc_node(node_id),
    rel             text        NOT NULL,
    strength        real        NOT NULL DEFAULT 0.1,
    tenant_id       uuid        NOT NULL,
    scope_id        uuid        NOT NULL,
    t_created       timestamptz NOT NULL DEFAULT now(),
    t_expired       timestamptz,
    t_valid         timestamptz NOT NULL,
    t_invalid       timestamptz,
    provenance      bytea[]     NOT NULL DEFAULT ARRAY[]::bytea[]
);

-- Current edges from a source, by relation. Partial index keeps it small.
CREATE INDEX nc_edge_src_rel_current_idx ON nc_edge (tenant_id, src, rel)
    WHERE t_expired IS NULL AND t_invalid IS NULL;

-- Current edges to a destination, by relation.
CREATE INDEX nc_edge_dst_rel_current_idx ON nc_edge (tenant_id, dst, rel)
    WHERE t_expired IS NULL AND t_invalid IS NULL;

-- Time-travel queries: find edges valid at a point in time.
CREATE INDEX nc_edge_tenant_src_valid_idx ON nc_edge (tenant_id, src, t_valid);
