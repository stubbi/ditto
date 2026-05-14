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
    Event, EventId, Receipt, ScopeId, SchemaVersion, Signature, Slot, TenantId,
};
use ditto_memory::search::{SearchQuery, SearchResult};
use ditto_memory::storage::{Storage, StorageError, StorageResult};

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
        tx.commit()
            .await
            .map_err(|e| StorageError::Other(format!("reset commit: {e}")))?;
        Ok(())
    }
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
