//! Ditto CLI — v0 smoke commands.
//!
//! Subcommands:
//!   migrate   apply Postgres schema migrations
//!   write     commit a single event (payload supplied as JSON arg)
//!   search    search the tenant's memory
//!   keygen    print a fresh Ed25519 install key (32-byte hex)
//!
//! Backend selection: if `--database-url` (or `DATABASE_URL`) is set, use
//! Postgres; otherwise the in-memory backend (write+search only, no
//! persistence — exists for smoke testing without a database).

use std::env;
use std::str::FromStr;

use anyhow::{Context, Result};
use chrono::Utc;
use clap::{Parser, Subcommand};

use ditto_core::{InstallKey, ScopeId, Slot, TenantId};
use ditto_memory::{InMemoryStorage, MemoryController, SearchMode, SearchQuery, Storage};
use ditto_storage_postgres::PostgresStorage;

#[derive(Parser, Debug)]
#[command(name = "ditto", version, about = "Ditto agent memory CLI (v0)")]
struct Cli {
    /// Postgres database URL. If unset, falls back to DATABASE_URL env, then
    /// to an ephemeral in-memory backend (no persistence).
    #[arg(long, global = true)]
    database_url: Option<String>,

    /// Hex-encoded 32-byte install secret. Generated fresh each run if unset.
    /// In production this is read from the install's keyring/vault.
    #[arg(long, global = true, env = "DITTO_INSTALL_SECRET_HEX")]
    install_secret_hex: Option<String>,

    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Apply pending Postgres migrations. Requires a database URL.
    Migrate,
    /// Commit one event.
    Write {
        #[arg(long)]
        tenant: String,
        #[arg(long)]
        scope: String,
        #[arg(long)]
        source: String,
        #[arg(long, default_value = "episodic_index")]
        slot: String,
        /// Payload as a JSON string. Example: '{"content":"hi"}'
        #[arg(long)]
        payload: String,
    },
    /// Search a tenant's memory.
    Search {
        #[arg(long)]
        tenant: String,
        #[arg(long)]
        query: String,
        #[arg(long, default_value = "10")]
        k: usize,
        #[arg(long, default_value = "standard")]
        mode: String,
    },
    /// Print a fresh 32-byte Ed25519 install secret (hex).
    Keygen,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();

    if matches!(cli.cmd, Cmd::Keygen) {
        let key = InstallKey::generate();
        println!("{}", hex::encode(key.secret_bytes()));
        return Ok(());
    }

    let install_key = match &cli.install_secret_hex {
        Some(s) => {
            let bytes = hex::decode(s).context("install secret is not valid hex")?;
            InstallKey::from_secret_bytes(&bytes).context("install secret must be 32 bytes")?
        }
        None => InstallKey::generate(),
    };

    let database_url = cli.database_url.or_else(|| env::var("DATABASE_URL").ok());

    match (cli.cmd, database_url) {
        (Cmd::Migrate, Some(url)) => {
            let storage = PostgresStorage::connect(&url).await?;
            storage.migrate().await?;
            println!("migrations applied");
            Ok(())
        }
        (Cmd::Migrate, None) => {
            anyhow::bail!("migrate requires --database-url or DATABASE_URL")
        }
        (cmd, Some(url)) => {
            let storage = PostgresStorage::connect(&url).await?;
            let ctrl = MemoryController::new(storage, install_key);
            run_cmd(cmd, &ctrl).await
        }
        (cmd, None) => {
            let storage = InMemoryStorage::new();
            let ctrl = MemoryController::new(storage, install_key);
            tracing::warn!("no database URL — using ephemeral in-memory backend");
            run_cmd(cmd, &ctrl).await
        }
    }
}

async fn run_cmd<S: Storage>(cmd: Cmd, ctrl: &MemoryController<S>) -> Result<()> {
    match cmd {
        Cmd::Migrate | Cmd::Keygen => unreachable!(),
        Cmd::Write {
            tenant,
            scope,
            source,
            slot,
            payload,
        } => {
            let tenant_id = TenantId::from_str(&tenant)?;
            let scope_id = ScopeId::from_str(&scope)?;
            let slot = parse_slot(&slot)?;
            let payload: serde_json::Value = serde_json::from_str(&payload)?;
            let receipt = ctrl
                .write(tenant_id, scope_id, source, slot, payload, Utc::now())
                .await?;
            println!("{}", serde_json::to_string_pretty(&receipt)?);
            Ok(())
        }
        Cmd::Search {
            tenant,
            query,
            k,
            mode,
        } => {
            let tenant_id = TenantId::from_str(&tenant)?;
            let mut q = SearchQuery::new(query, tenant_id);
            q.k = k;
            q.mode = parse_mode(&mode)?;
            let results = ctrl.search(&q).await?;
            println!("{}", serde_json::to_string_pretty(&results)?);
            Ok(())
        }
    }
}

fn parse_slot(s: &str) -> Result<Slot> {
    Ok(match s {
        "working" => Slot::Working,
        "episodic_index" | "episodic" => Slot::EpisodicIndex,
        "blob_store" | "blob" => Slot::BlobStore,
        "nc_graph" | "graph" => Slot::NcGraph,
        "nc_doc" | "doc" => Slot::NcDoc,
        "procedural" => Slot::Procedural,
        "reflective" => Slot::Reflective,
        other => anyhow::bail!("unknown slot: {other}"),
    })
}

fn parse_mode(s: &str) -> Result<SearchMode> {
    Ok(match s {
        "cheap" => SearchMode::Cheap,
        "standard" => SearchMode::Standard,
        "deep" => SearchMode::Deep,
        other => anyhow::bail!("unknown mode: {other}"),
    })
}
