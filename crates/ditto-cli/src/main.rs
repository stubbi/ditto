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
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;

use anyhow::{Context, Result};
use chrono::Utc;
use clap::{Parser, Subcommand};

use ditto_core::{InstallKey, ScopeId, Slot, TenantId};
use ditto_mcp::serve_stdio;
use ditto_memory::embedder::{DeterministicEmbedder, Embedder};
#[cfg(feature = "openai-embedder")]
use ditto_memory::embedder::openai::OpenAiEmbedder;
use ditto_memory::extractor::{Extractor, NoopExtractor, RuleExtractor};
use ditto_memory::{InMemoryStorage, MemoryController, SearchMode, SearchQuery, Storage};
use ditto_render::{LocalFilesystem, RenderJob};
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
    /// Render NC-graph state to NC-doc Markdown pages on disk.
    Render {
        #[arg(long)]
        tenant: String,
        /// Restrict to one scope. If omitted, all scopes for the tenant.
        #[arg(long)]
        scope: Option<String>,
        /// Output directory. Will be created if missing.
        #[arg(long)]
        out: PathBuf,
    },
    /// Run an MCP server on stdio. Speaks the Model Context Protocol so
    /// Claude Code / Cursor / Zed / Codex Desktop can use Ditto's memory.
    Serve {
        /// Embedder for hybrid retrieval. `none` (default) → BM25 only;
        /// `deterministic` → in-process hash-projection (tests + CI);
        /// `openai` → OpenAI text-embedding-3-small (reads OPENAI_API_KEY);
        /// `openrouter` → same model via OpenRouter (reads OPENROUTER_API_KEY).
        #[arg(long, default_value = "none")]
        embedder: String,
        /// Fact extractor for NC-graph auto-population. `none` (default)
        /// → graph stays empty unless code manually writes; `rule` →
        /// deterministic pattern matcher (lives_in / moved_to / works_at /
        /// allergic_to). LLM-driven extractor is a follow-up.
        #[arg(long, default_value = "none")]
        extractor: String,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    // Logs go to stderr — stdout is reserved for JSON-RPC framing when
    // running as an MCP server, and even non-serve commands print
    // structured output to stdout that callers parse.
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_ansi(false)
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
            let storage = Arc::new(PostgresStorage::connect(&url).await?);
            let ctrl = MemoryController::new_with_arc(storage.clone(), Arc::new(install_key));
            run_cmd(cmd, ctrl, storage).await
        }
        (cmd, None) => {
            let storage = Arc::new(InMemoryStorage::new());
            let ctrl = MemoryController::new_with_arc(storage.clone(), Arc::new(install_key));
            tracing::warn!("no database URL — using ephemeral in-memory backend");
            run_cmd(cmd, ctrl, storage).await
        }
    }
}

async fn run_cmd<S: Storage + 'static>(
    cmd: Cmd,
    ctrl: MemoryController<S>,
    storage: Arc<S>,
) -> Result<()> {
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
        Cmd::Render { tenant, scope, out } => {
            let tenant_id = TenantId::from_str(&tenant)?;
            let scope_id = scope.as_deref().map(ScopeId::from_str).transpose()?;
            let fs = Arc::new(LocalFilesystem::new(out.clone()));
            let job = RenderJob::new(storage, fs);
            let report = job.render(tenant_id, scope_id).await?;
            println!(
                "rendered: written={} unchanged={} removed={} into {}",
                report.pages_written,
                report.pages_unchanged,
                report.pages_removed,
                out.display()
            );
            Ok(())
        }
        Cmd::Serve {
            embedder,
            extractor,
        } => {
            // Hand control to the MCP server. Logs go to stderr (configured
            // earlier); stdio is reserved for the JSON-RPC framing.
            let _ = storage; // referenced via ctrl
            let ctrl = build_embedder(ctrl, &embedder)?;
            let ctrl = build_extractor(ctrl, &extractor)?;
            serve_stdio(Arc::new(ctrl)).await?;
            Ok(())
        }
    }
}

fn build_embedder<S: Storage + 'static>(
    ctrl: MemoryController<S>,
    selection: &str,
) -> Result<MemoryController<S>> {
    let embedder: Option<Arc<dyn Embedder>> = match selection {
        "none" => None,
        "deterministic" => Some(Arc::new(DeterministicEmbedder::new())),
        "openai" => {
            #[cfg(feature = "openai-embedder")]
            {
                let e = OpenAiEmbedder::from_env().map_err(|e| anyhow::anyhow!(e.to_string()))?;
                Some(Arc::new(e))
            }
            #[cfg(not(feature = "openai-embedder"))]
            {
                anyhow::bail!(
                    "openai embedder not compiled in (rebuild with --features openai-embedder)"
                );
            }
        }
        "openrouter" => {
            #[cfg(feature = "openai-embedder")]
            {
                let e = OpenAiEmbedder::from_env_openrouter()
                    .map_err(|e| anyhow::anyhow!(e.to_string()))?;
                Some(Arc::new(e))
            }
            #[cfg(not(feature = "openai-embedder"))]
            {
                anyhow::bail!(
                    "openrouter embedder not compiled in (rebuild with --features openai-embedder)"
                );
            }
        }
        other => anyhow::bail!("unknown embedder selection: {other}"),
    };
    Ok(match embedder {
        Some(e) => ctrl.with_embedder(e),
        None => ctrl,
    })
}

fn build_extractor<S: Storage + 'static>(
    ctrl: MemoryController<S>,
    selection: &str,
) -> Result<MemoryController<S>> {
    let extractor: Option<Arc<dyn Extractor>> = match selection {
        "none" => None,
        "noop" => Some(Arc::new(NoopExtractor)),
        "rule" => Some(Arc::new(RuleExtractor::new())),
        other => anyhow::bail!("unknown extractor selection: {other}"),
    };
    Ok(match extractor {
        Some(e) => ctrl.with_extractor(e),
        None => ctrl,
    })
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
