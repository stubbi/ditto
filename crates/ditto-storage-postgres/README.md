# ditto-storage-postgres

Postgres backend for Ditto. Implements `ditto_memory::Storage` via sqlx.

## v0 scope

- `episodic` table — pointers + payload + tsvector-generated `content_tsv` column for BM25 search.
- `receipt` table — SCITT-style signed-receipt log, per-(tenant, source) hash chain via `prev_event_id`.
- Search: tsvector BM25 on episodic `content`. pgvector HNSW and KG traversal come next.

## Migrations

Migrations live at the workspace root under `migrations/`. Apply via:

```bash
ditto migrate --database-url postgres://...
```

or programmatically via `PostgresStorage::migrate`.

Migration discipline: **additive only**. Columns are added with defaults; never dropped. Renamed → `_deprecated_` prefix, application stops reading them, column lives forever.

## Forthcoming

- `nc_node` / `nc_edge` bi-temporal KG tables
- `procedural` skill index
- pgvector HNSW indices for vector retrieval
- RLS policies (currently enforced at application layer)
- pg_advisory_lock for per-(tenant, source) write linearizability
- DiskANN indices when pgvector ships them GA
