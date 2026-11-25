use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use betterlite_db::ptu::store::sleeper::Sleeper;
use betterlite_db::queues::pod_lock::{LockCache, PodLockManager};
use betterlite_router::deployment_identity::spec::DeploymentSpec;
use tokio_util::task::TaskTracker;

use crate::health_cycle::endpoints::HealthCheckResult;
use crate::health_cycle::telemetry::InstrumentationContext;
use crate::health_persistence::{HealthCheckStore, HealthCycle, SaveRequest, save_background};
use crate::health_probe_settings::targeting::health_check_eligible;
use crate::model_deprecation::clock::UtcClock;

use super::health_state::HealthStateWriter;
use super::lease::WindowLease;
use super::manager::SharedHealthCheckManager;
use super::probe::{SharedHealthProbe, SharedProbeCall};
use super::results::HealthResultsCache;

pub const SAVE_JOB_ID: &str = "background_health_check_db_save";
pub const BACKGROUND_CHECKED_BY: &str = "background_health_check";
const SOURCE: &str = "proxy_background_loop";

// _run_background_health_check's inputs: the loop's settings, the shared manager when Redis is configured, and the devices the save and the router's health state need.
pub struct BackgroundHealthLoop<Store: HealthCheckStore, L: LockCache> {
    pub interval_seconds: i64,
    pub details: bool,
    pub max_concurrency: Option<i64>,
    pub manager: Option<Arc<SharedHealthCheckManager>>,
    pub probe: Arc<dyn SharedHealthProbe>,
    pub deployments: Option<Arc<Vec<DeploymentSpec>>>,
    pub results: Arc<HealthResultsCache>,
    pub store: Option<Arc<Store>>,
    pub lock: Option<Arc<PodLockManager<L>>>,
    pub health_state: Option<Arc<dyn HealthStateWriter>>,
    pub clock: Arc<dyn UtcClock + Send + Sync>,
    pub tasks: TaskTracker,
}

// The loop task B8540 spawns on the server's tracker: a zero or negative interval never starts it.
pub async fn background_loop<Store, L, S>(deps: BackgroundHealthLoop<Store, L>, mut sleeper: S)
where
    Store: HealthCheckStore,
    L: LockCache + 'static,
    S: Sleeper + Send,
{
    if deps.interval_seconds <= 0 {
        return;
    }
    let interval = u64::try_from(deps.interval_seconds).unwrap_or_default();
    let mut cycle = 0_u64;
    loop {
        cycle = cycle.wrapping_add(1);
        run_cycle(&deps, cycle, &mut sleeper).await;
        sleeper.sleep(Duration::from_secs(interval)).await;
    }
}

pub(crate) async fn run_cycle<Store, L, S>(
    deps: &BackgroundHealthLoop<Store, L>,
    cycle: u64,
    sleeper: &mut S,
) where
    Store: HealthCheckStore,
    L: LockCache + 'static,
    S: Sleeper + Send,
{
    let context = InstrumentationContext {
        enabled: true,
        cycle_id: format!("bg-{cycle}"),
        source: SOURCE.to_owned(),
    };
    let models = match deps.deployments.as_deref() {
        Some(list) => health_check_eligible(list.as_slice(), true),
        None => Vec::new(),
    };
    let result = cycle_result(deps, models.as_slice(), &context, sleeper).await;
    deps.results.publish(&result);
    persist_cycle(deps, models.as_slice(), &result).await;
    if let Some(writer) = &deps.health_state {
        writer.write(
            &result.healthy_endpoints,
            &result.unhealthy_endpoints,
            &result.exceptions_by_model_id,
        );
    }
}

// The check itself, shared through the manager when Redis is configured and direct otherwise, with the source's second try.
async fn cycle_result<Store, L, S>(
    deps: &BackgroundHealthLoop<Store, L>,
    models: &[DeploymentSpec],
    context: &InstrumentationContext,
    sleeper: &mut S,
) -> HealthCheckResult
where
    Store: HealthCheckStore,
    L: LockCache + 'static,
    S: Sleeper + Send,
{
    let call = SharedProbeCall {
        models,
        details: deps.details,
        max_concurrency: deps.max_concurrency,
        skip_disabled: true,
        instrumentation: Some(context),
    };
    let outcome = match &deps.manager {
        Some(manager) => {
            manager
                .perform_shared_health_check(call, deps.probe.as_ref(), sleeper)
                .await
        }
        None => deps.probe.probe(call).await,
    };
    match outcome {
        Ok(result) => result,
        Err(error) => {
            tracing::error!(
                "Error in shared health check, falling back to direct health check: {error}"
            );
            direct_result(deps.probe.as_ref(), call).await
        }
    }
}

async fn direct_result(
    probe: &dyn SharedHealthProbe,
    call: SharedProbeCall<'_>,
) -> HealthCheckResult {
    match probe.probe(call).await {
        Ok(result) => result,
        Err(error) => {
            tracing::error!("the direct health check failed: {error}");
            HealthCheckResult::default()
        }
    }
}

// _save_background_health_checks_to_db behind the window gate: who ran the cycle, and the one save the window allows.
async fn persist_cycle<Store, L>(
    deps: &BackgroundHealthLoop<Store, L>,
    models: &[DeploymentSpec],
    result: &HealthCheckResult,
) where
    Store: HealthCheckStore,
    L: LockCache + 'static,
{
    let cycle_data = HealthCycle {
        deployments: models,
        healthy: &result.healthy_endpoints,
        unhealthy: &result.unhealthy_endpoints,
    };
    let request = SaveRequest {
        start_time: deps.clock.now(),
        checked_by: Some(checked_by(deps)),
    };
    let save = save_background(
        deps.store.as_deref(),
        cycle_data,
        &request,
        deps.clock.as_ref(),
    );
    let saved = window_gated_save(
        deps.lock.as_ref(),
        &deps.tasks,
        Some(deps.interval_seconds),
        save,
    )
    .await;
    tracing::debug!(saved, "the background health check cycle saved or skipped");
}

// The source names the pod in a shared setup and the fixed background marker otherwise.
fn checked_by<Store, L>(deps: &BackgroundHealthLoop<Store, L>) -> &str
where
    Store: HealthCheckStore,
    L: LockCache,
{
    match &deps.manager {
        Some(manager) => manager.pod_id.as_str(),
        None => BACKGROUND_CHECKED_BY,
    }
}

// _window_gated_health_check_db_save: one save per window, kept by the completed save and released on failure or cancellation.
pub(crate) async fn window_gated_save<L, F>(
    lock: Option<&Arc<PodLockManager<L>>>,
    tasks: &TaskTracker,
    ttl: Option<i64>,
    save: F,
) -> bool
where
    L: LockCache + 'static,
    F: Future<Output = bool>,
{
    let Some(manager) = lock else {
        return save.await;
    };
    if manager.cache.is_none() {
        return save.await;
    }
    let acquired = manager.acquire_lock(SAVE_JOB_ID, ttl, false).await;
    if acquired != Some(true) {
        tracing::debug!(
            "background_health_check_db_save_skipped another pod persisted this window"
        );
        return false;
    }
    let lease = WindowLease::start(Arc::clone(manager), SAVE_JOB_ID, tasks);
    let persisted = save.await;
    if !persisted {
        tracing::warn!(
            "background_health_check_db_save_incomplete released the window lock so another pod can retry"
        );
        lease.release().await;
        return false;
    }
    lease.keep();
    true
}
