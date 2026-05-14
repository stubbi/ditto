//! Postgres implementation of `ditto-memory`'s `Storage` trait.
//!
//! v0 scope: episodic-index + receipt tables; BM25 search via `tsvector` on
//! `payload->>'content'`. Bi-temporal `nc_node`/`nc_edge`, procedural index,
//! and pgvector HNSW indices follow in subsequent migrations.
//!
//! Migrations live at the workspace root under `migrations/` and are applied
//! by `PostgresStorage::migrate`.

use std::str::FromStr;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::postgres::{PgPool, PgPoolOptions};
use sqlx::Row;

use ditto_core::{
    Blob, BlobHash, Edge, EdgeId, Event, EventId, NewEdge, NewNode, NewReflective, NewSkill, Node,
    NodeId, Receipt, Reflective, ReflectiveId, ScopeId, SchemaVersion, Signature, Skill, SkillId,
    SkillStatus, Slot, SupersedePolicy, TenantId,
};
use ditto_memory::search::{SearchQuery, SearchResult, VectorSearchQuery};
use ditto_memory::storage::{CascadeReport, Storage, StorageError, StorageResult};

pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../migrations");

pub struct PostgresStorage {
    pool: PgPool,
}

impl PostgresStorage {
    /// Connect to Postgres at `database_url`. Does not run migrations.
    pub async fn connect(database_url: &str) -> StorageResult<Self> {
        let pool = PgPoolOptions::new()
            .max_connections(8)
            .connect(database_url)
            .await
            .map_err(|e| StorageError::Other(format!("connect: {e}")))?;
        Ok(Self { pool })
    }

    /// Apply pending migrations.
    pub async fn migrate(&self) -> StorageResult<()> {
        MIGRATOR
            .run(&self.pool)
            .await
            .map_err(|e| StorageError::Other(format!("migrate: {e}")))?;
        Ok(())
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }
}

fn slot_to_str(slot: Slot) -> &'static str {
    match slot {
        Slot::Working => "working",
        Slot::EpisodicIndex => "episodic_index",
        Slot::BlobStore => "blob_store",
        Slot::NcGraph => "nc_graph",
        Slot::NcDoc => "nc_doc",
        Slot::Procedural => "procedural",
        Slot::Reflective => "reflective",
    }
}

fn str_to_slot(s: &str) -> StorageResult<Slot> {
    Ok(match s {
        "working" => Slot::Working,
        "episodic_index" => Slot::EpisodicIndex,
        "blob_store" => Slot::BlobStore,
        "nc_graph" => Slot::NcGraph,
        "nc_doc" => Slot::NcDoc,
        "procedural" => Slot::Procedural,
        "reflective" => Slot::Reflective,
        other => return Err(StorageError::Other(format!("unknown slot: {other}"))),
    })
}

#[async_trait]
impl Storage for PostgresStorage {
    async fn commit(&self, event: &Event, receipt: &Receipt) -> StorageResult<()> {
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| StorageError::Other(format!("begin: {e}")))?;

        // Idempotent insert via ON CONFLICT DO NOTHING on event_id PK.
        sqlx::query(
            r#"
            INSERT INTO episodic
                (event_id, prev_event_id, tenant_id, scope_id, source_id, slot,
                 payload, content, ts, schema_version)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
            ON CONFLICT (event_id) DO NOTHING
            "#,
        )
        .bind(event.event_id.0.as_slice())
        .bind(event.prev_event_id.map(|e| e.0.to_vec()))
        .bind(event.tenant_id.0)
        .bind(event.scope_id.0)
        .bind(&event.source_id)
        .bind(slot_to_str(event.slot))
        .bind(&event.payload)
        .bind(payload_content(&event.payload))
        .bind(event.timestamp)
        .bind(receipt.schema_version.as_u32() as i32)
        .execute(&mut *tx)
        .await
        .map_err(|e| StorageError::Other(format!("insert episodic: {e}")))?;

        sqlx::query(
            r#"
            INSERT INTO receipt
                (event_id, prev_event_id, tenant_id, source_id,
                 schema_version, signature, ts)
            VALUES ($1, $2, $3, $4, $5, $6, $7)
            ON CONFLICT (event_id) DO NOTHING
            "#,
        )
        .bind(receipt.event_id.0.as_slice())
        .bind(receipt.prev_event_id.map(|e| e.0.to_vec()))
        .bind(receipt.tenant_id.0)
        .bind(&receipt.source_id)
        .bind(receipt.schema_version.as_u32() as i32)
        .bind(receipt.signature.as_ref().map(|s| s.0.to_vec()))
        .bind(receipt.timestamp)
        .execute(&mut *tx)
        .await
        .map_err(|e| StorageError::Other(format!("insert receipt: {e}")))?;

        tx.commit()
            .await
            .map_err(|e| StorageError::Other(format!("commit: {e}")))?;
        Ok(())
    }

    async fn get_receipt(&self, event_id: &EventId) -> StorageResult<Option<Receipt>> {
        let row = sqlx::query(
            r#"
            SELECT event_id, prev_event_id, tenant_id, source_id, schema_version, signature, ts
            FROM receipt WHERE event_id = $1
            "#,
        )
        .bind(event_id.0.as_slice())
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| StorageError::Other(format!("get_receipt: {e}")))?;

        Ok(row.map(row_to_receipt).transpose()?)
    }

    async fn get_event(&self, event_id: &EventId) -> StorageResult<Option<Event>> {
        let row = sqlx::query(
            r#"
            SELECT event_id, prev_event_id, tenant_id, scope_id, source_id, slot, payload, ts
            FROM episodic WHERE event_id = $1
            "#,
        )
        .bind(event_id.0.as_slice())
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| StorageError::Other(format!("get_event: {e}")))?;

        Ok(row.map(row_to_event).transpose()?)
    }

    async fn search(&self, query: &SearchQuery) -> StorageResult<Vec<SearchResult>> {
        // v0: tsvector BM25 on `content` column. pgvector + KG comes next.
        let rows = sqlx::query(
            r#"
            SELECT event_id, payload, source_id, ts, slot,
                   ts_rank_cd(content_tsv, plainto_tsquery('simple', $2)) AS score
            FROM episodic
            WHERE tenant_id = $1
              AND ($3::uuid IS NULL OR scope_id = $3)
              AND ($4::text[] IS NULL OR source_id = ANY($4))
              AND content_tsv @@ plainto_tsquery('simple', $2)
            ORDER BY score DESC, ts DESC
            LIMIT $5
            "#,
        )
        .bind(query.tenant_id.0)
        .bind(&query.query)
        .bind(query.scope_id.map(|s| s.0))
        .bind(query.sources.as_deref())
        .bind(query.k as i32)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| StorageError::Other(format!("search: {e}")))?;

        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            let bytes: Vec<u8> = row.try_get("event_id").unwrap_or_default();
            let event_id = EventId::from_hex(&hex_lower(&bytes))
                .map_err(|e| StorageError::Other(format!("event_id decode: {e}")))?;
            let payload: serde_json::Value = row.try_get("payload").unwrap_or(Value::Null);
            let source_id: String = row.try_get("source_id").unwrap_or_default();
            let ts: DateTime<Utc> = row.try_get("ts").unwrap_or_else(|_| Utc::now());
            let slot_str: String = row.try_get("slot").unwrap_or_default();
            let score: f32 = row.try_get::<f32, _>("score").unwrap_or(0.0);
            out.push(SearchResult {
                event_id,
                content: payload_content(&payload),
                score,
                source_event_ids: vec![event_id],
                metadata: serde_json::json!({
                    "source_id": source_id,
                    "timestamp": ts,
                    "slot": slot_str,
                }),
            });
        }
        Ok(out)
    }

    async fn reset(&self, tenant_id: TenantId) -> StorageResult<()> {
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| StorageError::Other(format!("begin: {e}")))?;
        sqlx::query("DELETE FROM nc_edge WHERE tenant_id = $1")
            .bind(tenant_id.0)
            .execute(&mut *tx)
            .await
            .map_err(|e| StorageError::Other(format!("reset nc_edge: {e}")))?;
        sqlx::query("DELETE FROM nc_node WHERE tenant_id = $1")
            .bind(tenant_id.0)
            .execute(&mut *tx)
            .await
            .map_err(|e| StorageError::Other(format!("reset nc_node: {e}")))?;
        sqlx::query("DELETE FROM receipt WHERE tenant_id = $1")
            .bind(tenant_id.0)
            .execute(&mut *tx)
            .await
            .map_err(|e| StorageError::Other(format!("reset receipt: {e}")))?;
        sqlx::query("DELETE FROM episodic WHERE tenant_id = $1")
            .bind(tenant_id.0)
            .execute(&mut *tx)
            .await
            .map_err(|e| StorageError::Other(format!("reset episodic: {e}")))?;
        sqlx::query("DELETE FROM blob WHERE tenant_id = $1")
            .bind(tenant_id.0)
            .execute(&mut *tx)
            .await
            .map_err(|e| StorageError::Other(format!("reset blob: {e}")))?;
        sqlx::query("DELETE FROM procedural WHERE tenant_id = $1")
            .bind(tenant_id.0)
            .execute(&mut *tx)
            .await
            .map_err(|e| StorageError::Other(format!("reset procedural: {e}")))?;
        sqlx::query("DELETE FROM reflective WHERE tenant_id = $1")
            .bind(tenant_id.0)
            .execute(&mut *tx)
            .await
            .map_err(|e| StorageError::Other(format!("reset reflective: {e}")))?;
        sqlx::query("DELETE FROM labile_window WHERE tenant_id = $1")
            .bind(tenant_id.0)
            .execute(&mut *tx)
            .await
            .map_err(|e| StorageError::Other(format!("reset labile_window: {e}")))?;
        sqlx::query("DELETE FROM event_shadow WHERE tenant_id = $1")
            .bind(tenant_id.0)
            .execute(&mut *tx)
            .await
            .map_err(|e| StorageError::Other(format!("reset event_shadow: {e}")))?;
        tx.commit()
            .await
            .map_err(|e| StorageError::Other(format!("reset commit: {e}")))?;
        Ok(())
    }

    // --- NC-graph ---

    async fn insert_node(&self, node: NewNode) -> StorageResult<Node> {
        let provenance_bytes: Vec<Vec<u8>> = node.provenance.iter().map(|e| e.0.to_vec()).collect();
        let now = Utc::now();
        let row = sqlx::query(
            r#"
            INSERT INTO nc_node
                (node_id, tenant_id, scope_id, node_type, properties, t_created, provenance)
            VALUES ($1, $2, $3, $4, $5, $6, $7)
            RETURNING node_id, tenant_id, scope_id, node_type, properties, t_created, provenance
            "#,
        )
        .bind(node.node_id.0)
        .bind(node.tenant_id.0)
        .bind(node.scope_id.0)
        .bind(&node.node_type)
        .bind(&node.properties)
        .bind(now)
        .bind(&provenance_bytes)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| StorageError::Other(format!("insert_node: {e}")))?;
        row_to_node(row)
    }

    async fn assert_node(&self, node: NewNode) -> StorageResult<Node> {
        let provenance_bytes: Vec<Vec<u8>> = node.provenance.iter().map(|e| e.0.to_vec()).collect();
        let now = Utc::now();
        // ON CONFLICT (node_id) DO NOTHING + RETURNING returns 0 rows on conflict,
        // so we fall back to a SELECT for the existing row.
        sqlx::query(
            r#"
            INSERT INTO nc_node
                (node_id, tenant_id, scope_id, node_type, properties, t_created, provenance)
            VALUES ($1, $2, $3, $4, $5, $6, $7)
            ON CONFLICT (node_id) DO NOTHING
            "#,
        )
        .bind(node.node_id.0)
        .bind(node.tenant_id.0)
        .bind(node.scope_id.0)
        .bind(&node.node_type)
        .bind(&node.properties)
        .bind(now)
        .bind(&provenance_bytes)
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::Other(format!("assert_node insert: {e}")))?;

        match self.get_node(node.node_id).await? {
            Some(n) => Ok(n),
            None => Err(StorageError::Other("assert_node: node missing after insert".into())),
        }
    }

    async fn get_node(&self, node_id: NodeId) -> StorageResult<Option<Node>> {
        let row = sqlx::query(
            r#"SELECT node_id, tenant_id, scope_id, node_type, properties, t_created, provenance
               FROM nc_node WHERE node_id = $1"#,
        )
        .bind(node_id.0)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| StorageError::Other(format!("get_node: {e}")))?;
        Ok(row.map(row_to_node).transpose()?)
    }

    async fn insert_edge(&self, new_edge: NewEdge) -> StorageResult<Edge> {
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| StorageError::Other(format!("begin: {e}")))?;

        // Supersession runs in the same transaction so insert + invalidate are atomic.
        if let Some(policy) = new_edge.supersede {
            let dst_filter: Option<uuid::Uuid> = match policy {
                SupersedePolicy::AnyWithSameRelation => None,
                SupersedePolicy::SameSrcRelDst => Some(new_edge.dst.0),
            };
            sqlx::query(
                r#"
                UPDATE nc_edge
                SET t_invalid = $4, t_expired = now()
                WHERE tenant_id = $1
                  AND src = $2
                  AND rel = $3
                  AND t_expired IS NULL
                  AND t_invalid IS NULL
                  AND ($5::uuid IS NULL OR dst = $5)
                "#,
            )
            .bind(new_edge.tenant_id.0)
            .bind(new_edge.src.0)
            .bind(&new_edge.rel)
            .bind(new_edge.t_valid)
            .bind(dst_filter)
            .execute(&mut *tx)
            .await
            .map_err(|e| StorageError::Other(format!("supersede: {e}")))?;
        }

        let provenance_bytes: Vec<Vec<u8>> =
            new_edge.provenance.iter().map(|e| e.0.to_vec()).collect();
        let row = sqlx::query(
            r#"
            INSERT INTO nc_edge
                (edge_id, src, dst, rel, strength, tenant_id, scope_id,
                 t_created, t_expired, t_valid, t_invalid, provenance)
            VALUES ($1, $2, $3, $4, $5, $6, $7, now(), NULL, $8, $9, $10)
            RETURNING edge_id, src, dst, rel, strength, tenant_id, scope_id,
                      t_created, t_expired, t_valid, t_invalid, provenance
            "#,
        )
        .bind(new_edge.edge_id.0)
        .bind(new_edge.src.0)
        .bind(new_edge.dst.0)
        .bind(&new_edge.rel)
        .bind(new_edge.strength.unwrap_or(0.1))
        .bind(new_edge.tenant_id.0)
        .bind(new_edge.scope_id.0)
        .bind(new_edge.t_valid)
        .bind(new_edge.t_invalid)
        .bind(&provenance_bytes)
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| StorageError::Other(format!("insert_edge: {e}")))?;

        let edge = row_to_edge(row)?;
        tx.commit()
            .await
            .map_err(|e| StorageError::Other(format!("insert_edge commit: {e}")))?;
        Ok(edge)
    }

    async fn get_edge(&self, edge_id: EdgeId) -> StorageResult<Option<Edge>> {
        let row = sqlx::query(
            r#"SELECT edge_id, src, dst, rel, strength, tenant_id, scope_id,
                      t_created, t_expired, t_valid, t_invalid, provenance
               FROM nc_edge WHERE edge_id = $1"#,
        )
        .bind(edge_id.0)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| StorageError::Other(format!("get_edge: {e}")))?;
        Ok(row.map(row_to_edge).transpose()?)
    }

    async fn current_edges_from(
        &self,
        tenant_id: TenantId,
        src: NodeId,
        rel: Option<&str>,
    ) -> StorageResult<Vec<Edge>> {
        let rows = sqlx::query(
            r#"SELECT edge_id, src, dst, rel, strength, tenant_id, scope_id,
                      t_created, t_expired, t_valid, t_invalid, provenance
               FROM nc_edge
               WHERE tenant_id = $1 AND src = $2
                 AND t_expired IS NULL AND t_invalid IS NULL
                 AND ($3::text IS NULL OR rel = $3)
               ORDER BY t_created DESC"#,
        )
        .bind(tenant_id.0)
        .bind(src.0)
        .bind(rel)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| StorageError::Other(format!("current_edges_from: {e}")))?;
        rows.into_iter().map(row_to_edge).collect()
    }

    async fn current_edges_to(
        &self,
        tenant_id: TenantId,
        dst: NodeId,
        rel: Option<&str>,
    ) -> StorageResult<Vec<Edge>> {
        let rows = sqlx::query(
            r#"SELECT edge_id, src, dst, rel, strength, tenant_id, scope_id,
                      t_created, t_expired, t_valid, t_invalid, provenance
               FROM nc_edge
               WHERE tenant_id = $1 AND dst = $2
                 AND t_expired IS NULL AND t_invalid IS NULL
                 AND ($3::text IS NULL OR rel = $3)
               ORDER BY t_created DESC"#,
        )
        .bind(tenant_id.0)
        .bind(dst.0)
        .bind(rel)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| StorageError::Other(format!("current_edges_to: {e}")))?;
        rows.into_iter().map(row_to_edge).collect()
    }

    async fn edges_from_at(
        &self,
        tenant_id: TenantId,
        src: NodeId,
        t: DateTime<Utc>,
    ) -> StorageResult<Vec<Edge>> {
        let rows = sqlx::query(
            r#"SELECT edge_id, src, dst, rel, strength, tenant_id, scope_id,
                      t_created, t_expired, t_valid, t_invalid, provenance
               FROM nc_edge
               WHERE tenant_id = $1 AND src = $2
                 AND t_valid <= $3
                 AND (t_invalid IS NULL OR t_invalid > $3)
               ORDER BY t_valid DESC"#,
        )
        .bind(tenant_id.0)
        .bind(src.0)
        .bind(t)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| StorageError::Other(format!("edges_from_at: {e}")))?;
        rows.into_iter().map(row_to_edge).collect()
    }

    async fn invalidate_edge(
        &self,
        edge_id: EdgeId,
        t_invalid: DateTime<Utc>,
    ) -> StorageResult<()> {
        let res = sqlx::query("UPDATE nc_edge SET t_invalid = $2 WHERE edge_id = $1")
            .bind(edge_id.0)
            .bind(t_invalid)
            .execute(&self.pool)
            .await
            .map_err(|e| StorageError::Other(format!("invalidate_edge: {e}")))?;
        if res.rows_affected() == 0 {
            return Err(StorageError::Other(format!("edge {edge_id} not found")));
        }
        Ok(())
    }

    async fn list_nodes(
        &self,
        tenant_id: TenantId,
        scope_id: Option<ScopeId>,
    ) -> StorageResult<Vec<Node>> {
        let rows = sqlx::query(
            r#"SELECT node_id, tenant_id, scope_id, node_type, properties, t_created, provenance
               FROM nc_node
               WHERE tenant_id = $1 AND ($2::uuid IS NULL OR scope_id = $2)
               ORDER BY node_id"#,
        )
        .bind(tenant_id.0)
        .bind(scope_id.map(|s| s.0))
        .fetch_all(&self.pool)
        .await
        .map_err(|e| StorageError::Other(format!("list_nodes: {e}")))?;
        rows.into_iter().map(row_to_node).collect()
    }

    async fn edges_from_all_time(
        &self,
        tenant_id: TenantId,
        src: NodeId,
        rel: Option<&str>,
    ) -> StorageResult<Vec<Edge>> {
        let rows = sqlx::query(
            r#"SELECT edge_id, src, dst, rel, strength, tenant_id, scope_id,
                      t_created, t_expired, t_valid, t_invalid, provenance
               FROM nc_edge
               WHERE tenant_id = $1 AND src = $2
                 AND ($3::text IS NULL OR rel = $3)
               ORDER BY rel, t_valid, dst"#,
        )
        .bind(tenant_id.0)
        .bind(src.0)
        .bind(rel)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| StorageError::Other(format!("edges_from_all_time: {e}")))?;
        rows.into_iter().map(row_to_edge).collect()
    }

    async fn edges_to_all_time(
        &self,
        tenant_id: TenantId,
        dst: NodeId,
        rel: Option<&str>,
    ) -> StorageResult<Vec<Edge>> {
        let rows = sqlx::query(
            r#"SELECT edge_id, src, dst, rel, strength, tenant_id, scope_id,
                      t_created, t_expired, t_valid, t_invalid, provenance
               FROM nc_edge
               WHERE tenant_id = $1 AND dst = $2
                 AND ($3::text IS NULL OR rel = $3)
               ORDER BY rel, t_valid, src"#,
        )
        .bind(tenant_id.0)
        .bind(dst.0)
        .bind(rel)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| StorageError::Other(format!("edges_to_all_time: {e}")))?;
        rows.into_iter().map(row_to_edge).collect()
    }

    async fn put_blob(&self, tenant_id: TenantId, blob: &Blob) -> StorageResult<BlobHash> {
        let hash = blob.hash();
        let bytelen = i32::try_from(blob.bytes.len())
            .map_err(|_| StorageError::Other("blob exceeds i32 byte length".into()))?;
        // Idempotent on PK (tenant_id, content_hash). `ON CONFLICT DO NOTHING`
        // preserves first-writer's content_type — the second `put_blob` of
        // the same bytes by the same tenant is a no-op, matching InMemory.
        sqlx::query(
            r#"INSERT INTO blob (tenant_id, content_hash, content_type, payload, bytelen)
               VALUES ($1, $2, $3, $4, $5)
               ON CONFLICT (tenant_id, content_hash) DO NOTHING"#,
        )
        .bind(tenant_id.0)
        .bind(&hash.0[..])
        .bind(&blob.content_type)
        .bind(&blob.bytes)
        .bind(bytelen)
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::Other(format!("put_blob: {e}")))?;
        Ok(hash)
    }

    async fn get_blob(&self, tenant_id: TenantId, hash: BlobHash) -> StorageResult<Option<Blob>> {
        let row = sqlx::query(
            r#"SELECT content_type, payload
               FROM blob
               WHERE tenant_id = $1 AND content_hash = $2"#,
        )
        .bind(tenant_id.0)
        .bind(&hash.0[..])
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| StorageError::Other(format!("get_blob: {e}")))?;
        let Some(row) = row else { return Ok(None) };
        let content_type: String = row
            .try_get("content_type")
            .map_err(|e| StorageError::Other(format!("content_type col: {e}")))?;
        let payload: Vec<u8> = row
            .try_get("payload")
            .map_err(|e| StorageError::Other(format!("payload col: {e}")))?;
        Ok(Some(Blob::new(payload, content_type)))
    }

    async fn has_blob(&self, tenant_id: TenantId, hash: BlobHash) -> StorageResult<bool> {
        let row = sqlx::query(
            r#"SELECT 1 AS one
               FROM blob
               WHERE tenant_id = $1 AND content_hash = $2"#,
        )
        .bind(tenant_id.0)
        .bind(&hash.0[..])
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| StorageError::Other(format!("has_blob: {e}")))?;
        Ok(row.is_some())
    }

    async fn register_skill(&self, skill: NewSkill) -> StorageResult<Skill> {
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| StorageError::Other(format!("begin: {e}")))?;
        let existing = sqlx::query(
            r#"SELECT version, scope_id, path, last_used, tests_pass, status
               FROM procedural
               WHERE tenant_id = $1 AND skill_id = $2
               FOR UPDATE"#,
        )
        .bind(skill.tenant_id.0)
        .bind(&skill.skill_id.0)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|e| StorageError::Other(format!("register_skill lookup: {e}")))?;

        if let Some(row) = existing {
            let version: String = row
                .try_get("version")
                .map_err(|e| StorageError::Other(format!("version col: {e}")))?;
            if version != skill.version {
                return Err(StorageError::Other(format!(
                    "skill {} already registered with version {} (got {})",
                    skill.skill_id, version, skill.version
                )));
            }
            // Idempotent on identical re-register — return the existing row.
            tx.commit()
                .await
                .map_err(|e| StorageError::Other(format!("commit: {e}")))?;
            return self
                .get_skill(skill.tenant_id, &skill.skill_id)
                .await?
                .ok_or_else(|| StorageError::Other("skill vanished after upsert".into()));
        }

        sqlx::query(
            r#"INSERT INTO procedural
                 (tenant_id, skill_id, scope_id, version, path, last_used, tests_pass, status)
               VALUES ($1, $2, $3, $4, $5, NULL, NULL, 'active')"#,
        )
        .bind(skill.tenant_id.0)
        .bind(&skill.skill_id.0)
        .bind(skill.scope_id.0)
        .bind(&skill.version)
        .bind(&skill.path)
        .execute(&mut *tx)
        .await
        .map_err(|e| StorageError::Other(format!("register_skill insert: {e}")))?;
        tx.commit()
            .await
            .map_err(|e| StorageError::Other(format!("commit: {e}")))?;

        self.get_skill(skill.tenant_id, &skill.skill_id)
            .await?
            .ok_or_else(|| StorageError::Other("skill vanished after insert".into()))
    }

    async fn get_skill(
        &self,
        tenant_id: TenantId,
        skill_id: &SkillId,
    ) -> StorageResult<Option<Skill>> {
        let row = sqlx::query(
            r#"SELECT skill_id, scope_id, version, path, last_used, tests_pass, status
               FROM procedural
               WHERE tenant_id = $1 AND skill_id = $2"#,
        )
        .bind(tenant_id.0)
        .bind(&skill_id.0)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| StorageError::Other(format!("get_skill: {e}")))?;
        row.map(|r| row_to_skill(tenant_id, r)).transpose()
    }

    async fn list_skills(
        &self,
        tenant_id: TenantId,
        status_filter: Option<SkillStatus>,
    ) -> StorageResult<Vec<Skill>> {
        let rows = sqlx::query(
            r#"SELECT skill_id, scope_id, version, path, last_used, tests_pass, status
               FROM procedural
               WHERE tenant_id = $1
                 AND ($2::text IS NULL OR status = $2)
               ORDER BY skill_id"#,
        )
        .bind(tenant_id.0)
        .bind(status_filter.map(|s| s.as_str()))
        .fetch_all(&self.pool)
        .await
        .map_err(|e| StorageError::Other(format!("list_skills: {e}")))?;
        rows.into_iter()
            .map(|r| row_to_skill(tenant_id, r))
            .collect()
    }

    async fn mark_skill_used(
        &self,
        tenant_id: TenantId,
        skill_id: &SkillId,
        at: DateTime<Utc>,
    ) -> StorageResult<()> {
        let n = sqlx::query(
            r#"UPDATE procedural SET last_used = $3
               WHERE tenant_id = $1 AND skill_id = $2"#,
        )
        .bind(tenant_id.0)
        .bind(&skill_id.0)
        .bind(at)
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::Other(format!("mark_skill_used: {e}")))?;
        if n.rows_affected() == 0 {
            return Err(StorageError::Other(format!("skill not found: {skill_id}")));
        }
        Ok(())
    }

    async fn set_skill_tests_pass(
        &self,
        tenant_id: TenantId,
        skill_id: &SkillId,
        pass: f32,
    ) -> StorageResult<()> {
        let n = sqlx::query(
            r#"UPDATE procedural SET tests_pass = $3
               WHERE tenant_id = $1 AND skill_id = $2"#,
        )
        .bind(tenant_id.0)
        .bind(&skill_id.0)
        .bind(pass.clamp(0.0, 1.0))
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::Other(format!("set_skill_tests_pass: {e}")))?;
        if n.rows_affected() == 0 {
            return Err(StorageError::Other(format!("skill not found: {skill_id}")));
        }
        Ok(())
    }

    async fn set_skill_status(
        &self,
        tenant_id: TenantId,
        skill_id: &SkillId,
        status: SkillStatus,
    ) -> StorageResult<()> {
        let n = sqlx::query(
            r#"UPDATE procedural SET status = $3
               WHERE tenant_id = $1 AND skill_id = $2"#,
        )
        .bind(tenant_id.0)
        .bind(&skill_id.0)
        .bind(status.as_str())
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::Other(format!("set_skill_status: {e}")))?;
        if n.rows_affected() == 0 {
            return Err(StorageError::Other(format!("skill not found: {skill_id}")));
        }
        Ok(())
    }

    async fn insert_reflective(&self, new: NewReflective) -> StorageResult<Reflective> {
        let source_bytes: Vec<&[u8]> = new
            .source_event_ids
            .iter()
            .map(|e| e.0.as_slice())
            .collect();
        let consolidation = new.consolidation_receipt.map(|e| e.0.to_vec());
        let row = sqlx::query(
            r#"INSERT INTO reflective
                 (reflective_id, tenant_id, scope_id, content, confidence,
                  source_event_ids, consolidation_receipt, t_valid)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
               RETURNING reflective_id, tenant_id, scope_id, content, confidence,
                         source_event_ids, consolidation_receipt,
                         t_created, t_expired, t_valid, t_invalid"#,
        )
        .bind(new.reflective_id.0)
        .bind(new.tenant_id.0)
        .bind(new.scope_id.0)
        .bind(&new.content)
        .bind(new.confidence.clamp(0.0, 1.0))
        .bind(&source_bytes)
        .bind(consolidation.as_deref())
        .bind(new.t_valid)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| StorageError::Other(format!("insert_reflective: {e}")))?;
        row_to_reflective(row)
    }

    async fn get_reflective(
        &self,
        tenant_id: TenantId,
        reflective_id: ReflectiveId,
    ) -> StorageResult<Option<Reflective>> {
        let row = sqlx::query(
            r#"SELECT reflective_id, tenant_id, scope_id, content, confidence,
                      source_event_ids, consolidation_receipt,
                      t_created, t_expired, t_valid, t_invalid
               FROM reflective
               WHERE tenant_id = $1 AND reflective_id = $2"#,
        )
        .bind(tenant_id.0)
        .bind(reflective_id.0)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| StorageError::Other(format!("get_reflective: {e}")))?;
        row.map(row_to_reflective).transpose()
    }

    async fn current_reflective(
        &self,
        tenant_id: TenantId,
        scope_id: Option<ScopeId>,
    ) -> StorageResult<Vec<Reflective>> {
        let rows = sqlx::query(
            r#"SELECT reflective_id, tenant_id, scope_id, content, confidence,
                      source_event_ids, consolidation_receipt,
                      t_created, t_expired, t_valid, t_invalid
               FROM reflective
               WHERE tenant_id = $1
                 AND t_expired IS NULL AND t_invalid IS NULL
                 AND ($2::uuid IS NULL OR scope_id = $2)
               ORDER BY t_valid DESC, reflective_id"#,
        )
        .bind(tenant_id.0)
        .bind(scope_id.map(|s| s.0))
        .fetch_all(&self.pool)
        .await
        .map_err(|e| StorageError::Other(format!("current_reflective: {e}")))?;
        rows.into_iter().map(row_to_reflective).collect()
    }

    async fn list_reflective_all_time(
        &self,
        tenant_id: TenantId,
        scope_id: Option<ScopeId>,
    ) -> StorageResult<Vec<Reflective>> {
        let rows = sqlx::query(
            r#"SELECT reflective_id, tenant_id, scope_id, content, confidence,
                      source_event_ids, consolidation_receipt,
                      t_created, t_expired, t_valid, t_invalid
               FROM reflective
               WHERE tenant_id = $1
                 AND ($2::uuid IS NULL OR scope_id = $2)
               ORDER BY t_valid, reflective_id"#,
        )
        .bind(tenant_id.0)
        .bind(scope_id.map(|s| s.0))
        .fetch_all(&self.pool)
        .await
        .map_err(|e| StorageError::Other(format!("list_reflective_all_time: {e}")))?;
        rows.into_iter().map(row_to_reflective).collect()
    }

    async fn invalidate_reflective(
        &self,
        reflective_id: ReflectiveId,
        t_invalid: DateTime<Utc>,
    ) -> StorageResult<()> {
        // Idempotent on equal t_invalid: a second call with the same value
        // is a no-op rather than a conflict. Mirrors invalidate_edge.
        let n = sqlx::query(
            r#"UPDATE reflective
               SET t_invalid = $2
               WHERE reflective_id = $1
                 AND (t_invalid IS NULL OR t_invalid = $2)"#,
        )
        .bind(reflective_id.0)
        .bind(t_invalid)
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::Other(format!("invalidate_reflective: {e}")))?;
        if n.rows_affected() == 0 {
            return Err(StorageError::Other(format!(
                "reflective not found or already invalidated at a different time: {reflective_id}"
            )));
        }
        Ok(())
    }

    async fn put_embedding(
        &self,
        tenant_id: TenantId,
        event_id: EventId,
        embedding: &[f32],
    ) -> StorageResult<()> {
        // pgvector binds via its `Vec<f32>` text-form `[1.0, 2.0, ...]` — we
        // serialize once here rather than depending on the pgvector crate so
        // a follow-up swap to sqlx-native vector binding is a one-file change.
        let vector_lit = embedding_to_pg_literal(embedding);
        let n = sqlx::query(
            r#"UPDATE episodic
               SET embedding = $3::vector
               WHERE tenant_id = $1 AND event_id = $2"#,
        )
        .bind(tenant_id.0)
        .bind(&event_id.0[..])
        .bind(vector_lit)
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::Other(format!("put_embedding: {e}")))?;
        if n.rows_affected() == 0 {
            return Err(StorageError::Other(format!(
                "no episodic row to attach embedding to: {event_id}"
            )));
        }
        Ok(())
    }

    async fn delete_node_cascade(
        &self,
        tenant_id: TenantId,
        node_id: NodeId,
    ) -> StorageResult<CascadeReport> {
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| StorageError::Other(format!("begin: {e}")))?;
        let edges = sqlx::query(
            "DELETE FROM nc_edge WHERE tenant_id = $1 AND (src = $2 OR dst = $2)",
        )
        .bind(tenant_id.0)
        .bind(node_id.0)
        .execute(&mut *tx)
        .await
        .map_err(|e| StorageError::Other(format!("delete edges cascade: {e}")))?;
        let n = sqlx::query("DELETE FROM nc_node WHERE tenant_id = $1 AND node_id = $2")
            .bind(tenant_id.0)
            .bind(node_id.0)
            .execute(&mut *tx)
            .await
            .map_err(|e| StorageError::Other(format!("delete node: {e}")))?;
        tx.commit()
            .await
            .map_err(|e| StorageError::Other(format!("commit cascade: {e}")))?;
        Ok(CascadeReport {
            node_removed: n.rows_affected() > 0,
            edges_removed: u32::try_from(edges.rows_affected()).unwrap_or(u32::MAX),
        })
    }

    async fn delete_blob(&self, tenant_id: TenantId, hash: BlobHash) -> StorageResult<bool> {
        let n = sqlx::query(
            "DELETE FROM blob WHERE tenant_id = $1 AND content_hash = $2",
        )
        .bind(tenant_id.0)
        .bind(&hash.0[..])
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::Other(format!("delete_blob: {e}")))?;
        Ok(n.rows_affected() > 0)
    }

    async fn open_labile(
        &self,
        tenant_id: TenantId,
        event_id: EventId,
        until: DateTime<Utc>,
    ) -> StorageResult<()> {
        sqlx::query(
            r#"INSERT INTO labile_window (tenant_id, event_id, labile_until)
               VALUES ($1, $2, $3)
               ON CONFLICT (tenant_id, event_id) DO UPDATE
                 SET labile_until = GREATEST(labile_window.labile_until, EXCLUDED.labile_until)"#,
        )
        .bind(tenant_id.0)
        .bind(&event_id.0[..])
        .bind(until)
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::Other(format!("open_labile: {e}")))?;
        Ok(())
    }

    async fn is_labile(
        &self,
        tenant_id: TenantId,
        event_id: EventId,
        now: DateTime<Utc>,
    ) -> StorageResult<Option<DateTime<Utc>>> {
        let row = sqlx::query(
            r#"SELECT labile_until
               FROM labile_window
               WHERE tenant_id = $1 AND event_id = $2 AND labile_until > $3"#,
        )
        .bind(tenant_id.0)
        .bind(&event_id.0[..])
        .bind(now)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| StorageError::Other(format!("is_labile: {e}")))?;
        Ok(row.and_then(|r| r.try_get("labile_until").ok()))
    }

    async fn prune_expired_labile(&self, now: DateTime<Utc>) -> StorageResult<u32> {
        let n = sqlx::query("DELETE FROM labile_window WHERE labile_until <= $1")
            .bind(now)
            .execute(&self.pool)
            .await
            .map_err(|e| StorageError::Other(format!("prune_expired_labile: {e}")))?;
        Ok(u32::try_from(n.rows_affected()).unwrap_or(u32::MAX))
    }

    async fn write_shadow(
        &self,
        tenant_id: TenantId,
        original: EventId,
        shadow: EventId,
        authority: &str,
    ) -> StorageResult<()> {
        let result = sqlx::query(
            r#"INSERT INTO event_shadow (tenant_id, original, shadow, authority)
               VALUES ($1, $2, $3, $4)
               ON CONFLICT (tenant_id, original) DO NOTHING
               RETURNING 1"#,
        )
        .bind(tenant_id.0)
        .bind(&original.0[..])
        .bind(&shadow.0[..])
        .bind(authority)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| StorageError::Other(format!("write_shadow: {e}")))?;
        if result.is_none() {
            return Err(StorageError::Other(format!(
                "shadow already exists for {original}"
            )));
        }
        Ok(())
    }

    async fn lookup_shadow(
        &self,
        tenant_id: TenantId,
        original: EventId,
    ) -> StorageResult<Option<EventId>> {
        let row = sqlx::query(
            r#"SELECT shadow FROM event_shadow
               WHERE tenant_id = $1 AND original = $2"#,
        )
        .bind(tenant_id.0)
        .bind(&original.0[..])
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| StorageError::Other(format!("lookup_shadow: {e}")))?;
        match row {
            Some(r) => {
                let bytes: Vec<u8> = r
                    .try_get("shadow")
                    .map_err(|e| StorageError::Other(format!("shadow col: {e}")))?;
                Ok(Some(
                    EventId::from_hex(&hex_lower(&bytes))
                        .map_err(|e| StorageError::Other(format!("shadow decode: {e}")))?,
                ))
            }
            None => Ok(None),
        }
    }

    async fn get_salience(
        &self,
        tenant_id: TenantId,
        event_id: EventId,
    ) -> StorageResult<Option<f32>> {
        let row = sqlx::query(
            "SELECT salience FROM episodic WHERE tenant_id = $1 AND event_id = $2",
        )
        .bind(tenant_id.0)
        .bind(&event_id.0[..])
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| StorageError::Other(format!("get_salience: {e}")))?;
        Ok(row.and_then(|r| r.try_get("salience").ok()))
    }

    async fn set_salience(
        &self,
        tenant_id: TenantId,
        event_id: EventId,
        value: f32,
    ) -> StorageResult<f32> {
        let clamped = value.clamp(0.0, 1.0);
        let n = sqlx::query(
            r#"UPDATE episodic SET salience = $3
               WHERE tenant_id = $1 AND event_id = $2"#,
        )
        .bind(tenant_id.0)
        .bind(&event_id.0[..])
        .bind(clamped)
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::Other(format!("set_salience: {e}")))?;
        if n.rows_affected() == 0 {
            return Err(StorageError::Other(format!("event not found: {event_id}")));
        }
        Ok(clamped)
    }

    async fn bump_salience(
        &self,
        tenant_id: TenantId,
        event_id: EventId,
        delta: f32,
    ) -> StorageResult<f32> {
        let row = sqlx::query(
            r#"UPDATE episodic
               SET salience = GREATEST(0.0, LEAST(1.0, salience + $3))
               WHERE tenant_id = $1 AND event_id = $2
               RETURNING salience"#,
        )
        .bind(tenant_id.0)
        .bind(&event_id.0[..])
        .bind(delta)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| StorageError::Other(format!("bump_salience: {e}")))?;
        match row {
            Some(r) => r
                .try_get("salience")
                .map_err(|e| StorageError::Other(format!("salience col: {e}"))),
            None => Err(StorageError::Other(format!("event not found: {event_id}"))),
        }
    }

    async fn decay_salience(
        &self,
        tenant_id: TenantId,
        factor: f32,
    ) -> StorageResult<u32> {
        let factor = factor.clamp(0.0, 1.0);
        let n = sqlx::query(
            r#"UPDATE episodic SET salience = salience * $2
               WHERE tenant_id = $1 AND salience > 0.0"#,
        )
        .bind(tenant_id.0)
        .bind(factor)
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::Other(format!("decay_salience: {e}")))?;
        Ok(u32::try_from(n.rows_affected()).unwrap_or(u32::MAX))
    }

    async fn list_episodic(
        &self,
        tenant_id: TenantId,
        scope_id: Option<ScopeId>,
        limit: Option<usize>,
    ) -> StorageResult<Vec<Event>> {
        let rows = sqlx::query(
            r#"SELECT event_id, prev_event_id, tenant_id, scope_id, source_id, slot,
                      payload, ts, schema_version
               FROM episodic
               WHERE tenant_id = $1
                 AND ($2::uuid IS NULL OR scope_id = $2)
               ORDER BY ts, event_id
               LIMIT COALESCE($3::bigint, 1000000000)"#,
        )
        .bind(tenant_id.0)
        .bind(scope_id.map(|s| s.0))
        .bind(limit.map(|l| l as i64))
        .fetch_all(&self.pool)
        .await
        .map_err(|e| StorageError::Other(format!("list_episodic: {e}")))?;
        rows.into_iter().map(row_to_event).collect()
    }

    async fn search_vector(&self, query: &VectorSearchQuery) -> StorageResult<Vec<SearchResult>> {
        let vector_lit = embedding_to_pg_literal(&query.embedding);
        let sources = query.sources.as_deref().map(|s| s.to_vec());
        let rows = sqlx::query(
            r#"SELECT event_id, content, ts, source_id, slot,
                      1 - (embedding <=> $2::vector) AS similarity
               FROM episodic
               WHERE tenant_id = $1
                 AND embedding IS NOT NULL
                 AND ($3::uuid IS NULL OR scope_id = $3)
                 AND ($4::text[] IS NULL OR source_id = ANY($4))
               ORDER BY embedding <=> $2::vector
               LIMIT $5"#,
        )
        .bind(query.tenant_id.0)
        .bind(vector_lit)
        .bind(query.scope_id.map(|s| s.0))
        .bind(sources)
        .bind(query.k as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| StorageError::Other(format!("search_vector: {e}")))?;

        rows.into_iter()
            .map(|row| {
                let event_id_bytes: Vec<u8> = row
                    .try_get("event_id")
                    .map_err(|e| StorageError::Other(format!("event_id col: {e}")))?;
                let event_id = EventId::from_hex(&hex_lower(&event_id_bytes))
                    .map_err(|e| StorageError::Other(format!("event_id decode: {e}")))?;
                let content: Option<String> = row
                    .try_get("content")
                    .map_err(|e| StorageError::Other(format!("content col: {e}")))?;
                let ts: DateTime<Utc> = row
                    .try_get("ts")
                    .map_err(|e| StorageError::Other(format!("ts col: {e}")))?;
                let source_id: String = row
                    .try_get("source_id")
                    .map_err(|e| StorageError::Other(format!("source_id col: {e}")))?;
                let slot: String = row
                    .try_get("slot")
                    .map_err(|e| StorageError::Other(format!("slot col: {e}")))?;
                let similarity: f64 = row
                    .try_get("similarity")
                    .map_err(|e| StorageError::Other(format!("similarity col: {e}")))?;
                Ok(SearchResult {
                    event_id,
                    content: content.unwrap_or_default(),
                    score: similarity as f32,
                    source_event_ids: vec![event_id],
                    metadata: serde_json::json!({
                        "source_id": source_id,
                        "timestamp": ts,
                        "slot": slot,
                        "leg": "vector",
                    }),
                })
            })
            .collect()
    }
}

fn embedding_to_pg_literal(v: &[f32]) -> String {
    // pgvector text format: `[1,2,3,...]`. No spaces required. We use the
    // text format because it avoids depending on a pgvector-specific sqlx
    // adapter; binding the literal as `text` and casting to `vector` on the
    // server side is supported and idiomatic.
    let mut out = String::with_capacity(v.len() * 8 + 2);
    out.push('[');
    for (i, x) in v.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        // R-style minimal float — `1`, `0.5`, `-1.25`. avoid `1e0`.
        out.push_str(&format!("{x}"));
    }
    out.push(']');
    out
}

fn payload_content(payload: &Value) -> String {
    payload
        .get("content")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .unwrap_or_else(|| payload.to_string())
}

fn hex_lower(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn row_to_event(row: sqlx::postgres::PgRow) -> StorageResult<Event> {
    let event_id_bytes: Vec<u8> = row
        .try_get("event_id")
        .map_err(|e| StorageError::Other(format!("event_id col: {e}")))?;
    let event_id = EventId::from_hex(&hex_lower(&event_id_bytes))
        .map_err(|e| StorageError::Other(format!("event_id decode: {e}")))?;

    let prev_bytes: Option<Vec<u8>> = row
        .try_get("prev_event_id")
        .map_err(|e| StorageError::Other(format!("prev_event_id col: {e}")))?;
    let prev_event_id = prev_bytes
        .map(|b| EventId::from_hex(&hex_lower(&b)))
        .transpose()
        .map_err(|e| StorageError::Other(format!("prev_event_id decode: {e}")))?;

    let tenant: uuid::Uuid = row
        .try_get("tenant_id")
        .map_err(|e| StorageError::Other(format!("tenant_id col: {e}")))?;
    let scope: uuid::Uuid = row
        .try_get("scope_id")
        .map_err(|e| StorageError::Other(format!("scope_id col: {e}")))?;
    let source_id: String = row
        .try_get("source_id")
        .map_err(|e| StorageError::Other(format!("source_id col: {e}")))?;
    let slot_str: String = row
        .try_get("slot")
        .map_err(|e| StorageError::Other(format!("slot col: {e}")))?;
    let slot = str_to_slot(&slot_str)?;
    let payload: Value = row
        .try_get("payload")
        .map_err(|e| StorageError::Other(format!("payload col: {e}")))?;
    let ts: DateTime<Utc> = row
        .try_get("ts")
        .map_err(|e| StorageError::Other(format!("ts col: {e}")))?;

    Ok(Event {
        event_id,
        prev_event_id,
        tenant_id: TenantId(tenant),
        scope_id: ScopeId(scope),
        source_id,
        slot,
        payload,
        timestamp: ts,
    })
}

fn row_to_receipt(row: sqlx::postgres::PgRow) -> StorageResult<Receipt> {
    let event_id_bytes: Vec<u8> = row
        .try_get("event_id")
        .map_err(|e| StorageError::Other(format!("event_id col: {e}")))?;
    let event_id = EventId::from_hex(&hex_lower(&event_id_bytes))
        .map_err(|e| StorageError::Other(format!("event_id decode: {e}")))?;

    let prev_bytes: Option<Vec<u8>> = row
        .try_get("prev_event_id")
        .map_err(|e| StorageError::Other(format!("prev_event_id col: {e}")))?;
    let prev_event_id = prev_bytes
        .map(|b| EventId::from_hex(&hex_lower(&b)))
        .transpose()
        .map_err(|e| StorageError::Other(format!("prev_event_id decode: {e}")))?;

    let tenant: uuid::Uuid = row
        .try_get("tenant_id")
        .map_err(|e| StorageError::Other(format!("tenant_id col: {e}")))?;
    let source_id: String = row
        .try_get("source_id")
        .map_err(|e| StorageError::Other(format!("source_id col: {e}")))?;
    let schema_version: i32 = row
        .try_get("schema_version")
        .map_err(|e| StorageError::Other(format!("schema_version col: {e}")))?;
    let sig_bytes: Option<Vec<u8>> = row
        .try_get("signature")
        .map_err(|e| StorageError::Other(format!("signature col: {e}")))?;
    let signature = sig_bytes
        .map(|b| Signature::from_hex(&hex_lower(&b)))
        .transpose()
        .map_err(|e| StorageError::Other(format!("signature decode: {e}")))?;
    let ts: DateTime<Utc> = row
        .try_get("ts")
        .map_err(|e| StorageError::Other(format!("ts col: {e}")))?;

    Ok(Receipt {
        event_id,
        prev_event_id,
        tenant_id: TenantId(tenant),
        source_id,
        timestamp: ts,
        schema_version: SchemaVersion(schema_version as u32),
        signature,
    })
}

#[allow(dead_code)]
fn parse_event_id(s: &str) -> StorageResult<EventId> {
    EventId::from_str(s).map_err(|e| StorageError::Other(format!("event_id parse: {e}")))
}

fn row_to_node(row: sqlx::postgres::PgRow) -> StorageResult<Node> {
    let node_id: uuid::Uuid = row
        .try_get("node_id")
        .map_err(|e| StorageError::Other(format!("node_id col: {e}")))?;
    let tenant_id: uuid::Uuid = row
        .try_get("tenant_id")
        .map_err(|e| StorageError::Other(format!("tenant_id col: {e}")))?;
    let scope_id: uuid::Uuid = row
        .try_get("scope_id")
        .map_err(|e| StorageError::Other(format!("scope_id col: {e}")))?;
    let node_type: String = row
        .try_get("node_type")
        .map_err(|e| StorageError::Other(format!("node_type col: {e}")))?;
    let properties: Value = row
        .try_get("properties")
        .map_err(|e| StorageError::Other(format!("properties col: {e}")))?;
    let t_created: DateTime<Utc> = row
        .try_get("t_created")
        .map_err(|e| StorageError::Other(format!("t_created col: {e}")))?;
    let provenance_bytes: Vec<Vec<u8>> = row
        .try_get("provenance")
        .map_err(|e| StorageError::Other(format!("provenance col: {e}")))?;
    let provenance = provenance_bytes
        .into_iter()
        .map(|b| EventId::from_hex(&hex_lower(&b)))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| StorageError::Other(format!("provenance decode: {e}")))?;

    Ok(Node {
        node_id: NodeId(node_id),
        tenant_id: TenantId(tenant_id),
        scope_id: ScopeId(scope_id),
        node_type,
        properties,
        t_created,
        provenance,
    })
}

fn row_to_edge(row: sqlx::postgres::PgRow) -> StorageResult<Edge> {
    let edge_id: uuid::Uuid = row
        .try_get("edge_id")
        .map_err(|e| StorageError::Other(format!("edge_id col: {e}")))?;
    let src: uuid::Uuid = row
        .try_get("src")
        .map_err(|e| StorageError::Other(format!("src col: {e}")))?;
    let dst: uuid::Uuid = row
        .try_get("dst")
        .map_err(|e| StorageError::Other(format!("dst col: {e}")))?;
    let rel: String = row
        .try_get("rel")
        .map_err(|e| StorageError::Other(format!("rel col: {e}")))?;
    let strength: f32 = row
        .try_get("strength")
        .map_err(|e| StorageError::Other(format!("strength col: {e}")))?;
    let tenant_id: uuid::Uuid = row
        .try_get("tenant_id")
        .map_err(|e| StorageError::Other(format!("tenant_id col: {e}")))?;
    let scope_id: uuid::Uuid = row
        .try_get("scope_id")
        .map_err(|e| StorageError::Other(format!("scope_id col: {e}")))?;
    let t_created: DateTime<Utc> = row
        .try_get("t_created")
        .map_err(|e| StorageError::Other(format!("t_created col: {e}")))?;
    let t_expired: Option<DateTime<Utc>> = row
        .try_get("t_expired")
        .map_err(|e| StorageError::Other(format!("t_expired col: {e}")))?;
    let t_valid: DateTime<Utc> = row
        .try_get("t_valid")
        .map_err(|e| StorageError::Other(format!("t_valid col: {e}")))?;
    let t_invalid: Option<DateTime<Utc>> = row
        .try_get("t_invalid")
        .map_err(|e| StorageError::Other(format!("t_invalid col: {e}")))?;
    let provenance_bytes: Vec<Vec<u8>> = row
        .try_get("provenance")
        .map_err(|e| StorageError::Other(format!("provenance col: {e}")))?;
    let provenance = provenance_bytes
        .into_iter()
        .map(|b| EventId::from_hex(&hex_lower(&b)))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| StorageError::Other(format!("provenance decode: {e}")))?;

    Ok(Edge {
        edge_id: EdgeId(edge_id),
        src: NodeId(src),
        dst: NodeId(dst),
        rel,
        strength,
        tenant_id: TenantId(tenant_id),
        scope_id: ScopeId(scope_id),
        t_created,
        t_expired,
        t_valid,
        t_invalid,
        provenance,
    })
}

fn row_to_skill(tenant_id: TenantId, row: sqlx::postgres::PgRow) -> StorageResult<Skill> {
    use std::str::FromStr;
    let skill_id: String = row
        .try_get("skill_id")
        .map_err(|e| StorageError::Other(format!("skill_id col: {e}")))?;
    let scope_id: uuid::Uuid = row
        .try_get("scope_id")
        .map_err(|e| StorageError::Other(format!("scope_id col: {e}")))?;
    let version: String = row
        .try_get("version")
        .map_err(|e| StorageError::Other(format!("version col: {e}")))?;
    let path: String = row
        .try_get("path")
        .map_err(|e| StorageError::Other(format!("path col: {e}")))?;
    let last_used: Option<DateTime<Utc>> = row
        .try_get("last_used")
        .map_err(|e| StorageError::Other(format!("last_used col: {e}")))?;
    let tests_pass: Option<f32> = row
        .try_get("tests_pass")
        .map_err(|e| StorageError::Other(format!("tests_pass col: {e}")))?;
    let status_str: String = row
        .try_get("status")
        .map_err(|e| StorageError::Other(format!("status col: {e}")))?;
    let status = SkillStatus::from_str(&status_str)
        .map_err(|e| StorageError::Other(format!("status decode: {e}")))?;
    Ok(Skill {
        skill_id: SkillId(skill_id),
        tenant_id,
        scope_id: ScopeId(scope_id),
        version,
        path,
        last_used,
        tests_pass,
        status,
    })
}

fn row_to_reflective(row: sqlx::postgres::PgRow) -> StorageResult<Reflective> {
    let reflective_id: uuid::Uuid = row
        .try_get("reflective_id")
        .map_err(|e| StorageError::Other(format!("reflective_id col: {e}")))?;
    let tenant_id: uuid::Uuid = row
        .try_get("tenant_id")
        .map_err(|e| StorageError::Other(format!("tenant_id col: {e}")))?;
    let scope_id: uuid::Uuid = row
        .try_get("scope_id")
        .map_err(|e| StorageError::Other(format!("scope_id col: {e}")))?;
    let content: String = row
        .try_get("content")
        .map_err(|e| StorageError::Other(format!("content col: {e}")))?;
    let confidence: f32 = row
        .try_get("confidence")
        .map_err(|e| StorageError::Other(format!("confidence col: {e}")))?;
    let source_bytes: Vec<Vec<u8>> = row
        .try_get("source_event_ids")
        .map_err(|e| StorageError::Other(format!("source_event_ids col: {e}")))?;
    let source_event_ids: Vec<EventId> = source_bytes
        .into_iter()
        .map(|b| EventId::from_hex(&hex_lower(&b)))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| StorageError::Other(format!("source_event_ids decode: {e}")))?;
    let cons_bytes: Option<Vec<u8>> = row
        .try_get("consolidation_receipt")
        .map_err(|e| StorageError::Other(format!("consolidation_receipt col: {e}")))?;
    let consolidation_receipt = cons_bytes
        .map(|b| EventId::from_hex(&hex_lower(&b)))
        .transpose()
        .map_err(|e| StorageError::Other(format!("consolidation_receipt decode: {e}")))?;
    let t_created: DateTime<Utc> = row
        .try_get("t_created")
        .map_err(|e| StorageError::Other(format!("t_created col: {e}")))?;
    let t_expired: Option<DateTime<Utc>> = row
        .try_get("t_expired")
        .map_err(|e| StorageError::Other(format!("t_expired col: {e}")))?;
    let t_valid: DateTime<Utc> = row
        .try_get("t_valid")
        .map_err(|e| StorageError::Other(format!("t_valid col: {e}")))?;
    let t_invalid: Option<DateTime<Utc>> = row
        .try_get("t_invalid")
        .map_err(|e| StorageError::Other(format!("t_invalid col: {e}")))?;
    Ok(Reflective {
        reflective_id: ReflectiveId(reflective_id),
        tenant_id: TenantId(tenant_id),
        scope_id: ScopeId(scope_id),
        content,
        confidence,
        source_event_ids,
        consolidation_receipt,
        t_created,
        t_expired,
        t_valid,
        t_invalid,
    })
}
