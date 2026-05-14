-- Dense retrieval — pgvector.
--
-- Adds a fixed-dim embedding column to episodic + an HNSW cosine index.
-- The dimension (1536) matches OpenAI text-embedding-3-small at full
-- resolution and what `ditto_memory::embedder::EMBEDDING_DIM` exports.
-- Adapters that produce a different native dimension are expected to
-- project (Matryoshka truncate or pad) before INSERT, so the schema stays
-- single-dim.
--
-- The HNSW index is built lazily on first query (pgvector default); the
-- `vector_cosine_ops` operator class means we search by cosine distance,
-- which the controller converts to similarity via `1 - distance`.
--
-- Operational requirement: the cluster must have the pgvector extension
-- available. Hosted Postgres (Supabase, Neon, AWS RDS 15.4+) ships it; a
-- vanilla `apt install postgresql` does not. The CREATE EXTENSION below
-- will fail with a clear message if it's missing.

CREATE EXTENSION IF NOT EXISTS vector;

ALTER TABLE episodic
    ADD COLUMN embedding vector(1536);

-- HNSW index — log-N retrieval at ~95% recall@10 with default params.
-- m=16 and ef_construction=64 are pgvector defaults; tune per workload.
CREATE INDEX episodic_embedding_hnsw_idx
    ON episodic
    USING hnsw (embedding vector_cosine_ops);

-- A separate tenant-prefixed btree so the planner can use it for filtered
-- vector searches (WHERE tenant_id = $1 ORDER BY embedding <=> $2). HNSW
-- does not currently support filtering inside the index walk; the planner
-- combines this index with the HNSW one via re-ranking.
CREATE INDEX episodic_tenant_embedding_idx
    ON episodic (tenant_id)
    WHERE embedding IS NOT NULL;
