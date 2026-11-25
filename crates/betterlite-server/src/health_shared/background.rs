use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use betterlite_db::ptu::store::sleeper::Sleeper;
use betterlite_db::queues::pod_lock::{LockCache, PodLockManager};
use betterlite_router::deployment_identity::spec::DeploymentSpec;
use tokio::time::timeout;
use tokio_util::task::TaskTracker;

use crate::health_cycle::endpoints::HealthCheckResult;
use crate::health_cycle::telemetry::InstrumentationContext;
use crate::health_persistence::{HealthCheckStore, HealthCycle, SaveRequest, save_background};
use crate::health_probe_settings::targeting::health_check_eligible;
use crate::model_deprecation::clock::UtcClock;

use super::error::SharedHealthError;
use super::health_state::HealthStateWriter;
use super::lease::WindowLease;
use super::manager::SharedHealthCheckManager;
use super::probe::{SharedHealthProbe, SharedProbeCall};
use super::results::HealthResultsCache;

pub const SAVE_JOB_ID: &str = "background_health_check_db_save";
pub const BACKGROUND_CHECKED_BY: &str = "background_health_check";
pub const SAVE_DEADLINE: Duration = Duration::from_secs(60);
const SOURCE: &str = "proxy_background_loop";

// llm_model_list as the loop reads it: LiteLLM deep-copies the current list at the top of every cycle, so a deployment added, removed or disabled after startup reaches the next probe.
pub trait DeploymentSource: Send + Sync {
    fn snapshot(&self) -> Option<Vec<DeploymentSpec>>;
}

// _run_background_health_check's inputs: the loop's settings, the shared manager when Redis is configured, and the devices the save and the router's health state need.
pub struct BackgroundHealthLoop<Store: HealthCheckStore, L: LockCache> {
    pub interval_seconds: i64,
    pub details: bool,
    pub max_concurrency: Option<i64>,
    pub manager: Option<Arc<SharedHealthCheckManager>>,
    pub probe: Arc<dyn SharedHealthProbe>,
    pub deployments: Option<Arc<dyn DeploymentSource>>,
    pub results: Arc<HealthResultsCache>,
    pub store: Option<Arc<Store>>,
    pub lock: Option<Arc<PodLockManager<L>>>,
    pub health_state: Option<Arc<dyn HealthStateWriter>>,
    pub clock: Arc<dyn UtcClock + Send + Sync>,
    pub save_deadline: Duration,
    pub tasks: TaskTracker,
}

// The loop task B8540 spawns on the server's tracker: a zero or negative interval never starts it, and a closed tracker stops it once the cycle in flight ends.
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
    while !deps.tasks.is_closed() {
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
    let deployment_list = deps
        .deployments
        .as_ref()
        .and_then(|source| source.snapshot());
    let models = deployment_list
        .as_deref()
        .map_or_else(Vec::new, |list| health_check_eligible(list, true));
    let result = match cycle_result(deps, models.as_slice(), &context, sleeper).await {
        Ok(result) => result,
        Err(error) => {
            tracing::error!(
                "the background health check cycle failed, keeping the previous results: {error}"
            );
            return;
        }
    };
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
) -> Result<HealthCheckResult, SharedHealthError>
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
    match &deps.manager {
        Some(manager) => {
            match manager
                .perform_shared_health_check(call, deps.probe.as_ref(), sleeper)
                .await
            {
                Ok(result) => Ok(result),
                Err(error) => {
                    tracing::error!(
                        "Error in shared health check, falling back to direct health check: {error}"
                    );
                    deps.probe.probe(call).await
                }
            }
        }
        None => deps.probe.probe(call).await,
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
        checked_by: Some(
            deps.manager
                .as_ref()
                .map_or(BACKGROUND_CHECKED_BY, |manager| manager.pod_id.as_str()),
        ),
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
        deps.save_deadline,
    )
    .await;
    tracing::debug!(saved, "the background health check cycle saved or skipped");
}

// _window_gated_health_check_db_save: one save per window, kept by the completed save and released on failure, timeout or cancellation.
pub(crate) async fn window_gated_save<L, F>(
    lock: Option<&Arc<PodLockManager<L>>>,
    tasks: &TaskTracker,
    ttl: Option<i64>,
    save: F,
    deadline: Duration,
) -> bool
where
    L: LockCache + 'static,
    F: Future<Output = bool>,
{
    let Some(manager) = lock else {
        return saved_within_deadline(save, deadline).await;
    };
    if manager.cache.is_none() {
        return saved_within_deadline(save, deadline).await;
    }
    let acquired = manager.acquire_lock(SAVE_JOB_ID, ttl, false).await;
    if acquired != Some(true) {
        tracing::debug!(
            "background_health_check_db_save_skipped another pod persisted this window"
        );
        return false;
    }
    let lease = WindowLease::start(Arc::clone(manager), SAVE_JOB_ID, tasks);
    let persisted = saved_within_deadline(save, deadline).await;
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

// A save that never returns is cut at the deadline, as a failed save: the caller releases the window lock and the next cycle tries again.
async fn saved_within_deadline<F>(save: F, deadline: Duration) -> bool
where
    F: Future<Output = bool>,
{
    timeout(deadline, save).await.unwrap_or_else(|_elapsed| {
        tracing::error!("background_health_check_db_save exceeded its {deadline:?} deadline");
        false
    })
}
