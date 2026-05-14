//! Long-sleep background scheduler.
//!
//! `MemoryController::consolidate(LongSleep)` is the per-tenant pass; this
//! module wraps it in a tokio task that fires at a configured interval over
//! a caller-provided tenant set. The architecture lists Long Sleep as the
//! third cadence (Ripple ≤200ms / Dream / LongSleep) — the slow, periodic
//! sweep that decays salience, prunes expired labile windows, and (later)
//! handles cold-subgraph archival and spaced-retrieval self-testing.
//!
//! v0 delivers the decay + prune half. The scheduler is intentionally a
//! thin loop — production deployments will pick the tenant set from a real
//! source (a tenant table, a topic, a heartbeat); the trait surface here is
//! `Fn() -> Vec<TenantId>` so the storage backend doesn't need a
//! `list_tenants` method (which would otherwise leak per-tenant data across
//! the multi-tenancy isolation boundary anyway).

use std::sync::Arc;
use std::time::Duration;

use tokio::task::JoinHandle;

use ditto_core::TenantId;

use crate::controller::{ConsolidationMode, ConsolidationReport, MemoryController};
use crate::storage::Storage;

/// Configuration knobs for the long-sleep scheduler.
#[derive(Clone, Debug)]
pub struct LongSleepConfig {
    /// How long to wait between sweeps. Default 1 hour.
    pub interval: Duration,
    /// Wait this long before the first sweep. Default 1 minute — gives
    /// the rest of the process time to come up before the first decay.
    pub startup_delay: Duration,
}

impl Default for LongSleepConfig {
    fn default() -> Self {
        Self {
            interval: Duration::from_secs(3600),
            startup_delay: Duration::from_secs(60),
        }
    }
}

/// Outcome of a single tick across all tenants in a sweep. Useful for
/// tests and for surfacing scheduler health in operational dashboards.
#[derive(Clone, Debug)]
pub struct LongSleepTick {
    pub reports: Vec<ConsolidationReport>,
    pub errors: Vec<(TenantId, String)>,
}

/// Spawns a tokio task that runs `consolidate(LongSleep)` over each
/// tenant returned by `tenants` once per `config.interval`. Returns the
/// `JoinHandle` so the caller can shut down on drop.
///
/// The tenant provider is consulted on each tick, so adding or removing
/// tenants between ticks is a no-op until the next firing.
pub struct LongSleepScheduler<S: Storage + 'static> {
    controller: Arc<MemoryController<S>>,
    config: LongSleepConfig,
}

impl<S: Storage + 'static> LongSleepScheduler<S> {
    pub fn new(controller: Arc<MemoryController<S>>, config: LongSleepConfig) -> Self {
        Self { controller, config }
    }

    /// Run a single tick synchronously. Used by tests and by callers who
    /// want manual control over scheduling (e.g., a job runner that fires
    /// long-sleep on cron rather than a self-managed tokio task).
    pub async fn tick<I>(&self, tenants: I) -> LongSleepTick
    where
        I: IntoIterator<Item = TenantId>,
    {
        let mut out = LongSleepTick {
            reports: Vec::new(),
            errors: Vec::new(),
        };
        for tenant in tenants {
            match self
                .controller
                .consolidate(tenant, None, ConsolidationMode::LongSleep)
                .await
            {
                Ok(r) => out.reports.push(r),
                Err(e) => out.errors.push((tenant, e.to_string())),
            }
        }
        out
    }

    /// Spawn the scheduler into the current tokio runtime. Drop the
    /// handle (or `abort` it) to stop.
    pub fn spawn<F>(self, tenants: F) -> JoinHandle<()>
    where
        F: Fn() -> Vec<TenantId> + Send + Sync + 'static,
    {
        let LongSleepScheduler { controller, config } = self;
        tokio::spawn(async move {
            tokio::time::sleep(config.startup_delay).await;
            let mut interval = tokio::time::interval(config.interval);
            // Tick-once on startup so the first iteration runs immediately
            // after the startup delay (not after a full interval).
            interval.tick().await;
            loop {
                let snapshot = tenants();
                for tenant in snapshot {
                    if let Err(e) = controller
                        .consolidate(tenant, None, ConsolidationMode::LongSleep)
                        .await
                    {
                        tracing::warn!(
                            tenant = %tenant,
                            error = %e,
                            "long sleep tick failed"
                        );
                    }
                }
                interval.tick().await;
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use chrono::Utc;
    use ditto_core::{InstallKey, Slot};
    use serde_json::json;

    use crate::in_memory::InMemoryStorage;

    use super::*;

    fn ctrl() -> Arc<MemoryController<InMemoryStorage>> {
        Arc::new(MemoryController::new(
            InMemoryStorage::new(),
            InstallKey::generate(),
        ))
    }

    #[tokio::test]
    async fn tick_decays_salience_and_returns_report_per_tenant() {
        let c = ctrl();
        let tenant = TenantId::new();
        let scope = ditto_core::ScopeId::new();
        c.write(
            tenant,
            scope,
            "s",
            Slot::EpisodicIndex,
            json!({"content": "ev"}),
            Utc::now(),
        )
        .await
        .unwrap();
        // Pump salience above the default 0.5 so we can see decay.
        let event = c
            .storage()
            .list_episodic(tenant, None, Some(1))
            .await
            .unwrap()[0]
            .clone();
        c.set_salience(tenant, event.event_id, 1.0).await.unwrap();

        let sched = LongSleepScheduler::new(c.clone(), LongSleepConfig::default());
        let tick = sched.tick(vec![tenant]).await;
        assert_eq!(tick.reports.len(), 1);
        assert!(tick.errors.is_empty());
        assert!(!tick.reports[0].stub);

        let after = c
            .get_salience(tenant, event.event_id)
            .await
            .unwrap()
            .unwrap();
        assert!(after < 1.0, "expected decay; got {after}");
    }

    #[tokio::test]
    async fn tick_handles_empty_tenant_set() {
        let c = ctrl();
        let sched = LongSleepScheduler::new(c, LongSleepConfig::default());
        let tick = sched.tick(Vec::<TenantId>::new()).await;
        assert!(tick.reports.is_empty());
        assert!(tick.errors.is_empty());
    }
}
