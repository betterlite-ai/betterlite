use std::future::Future;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_router::catalog_updates::catalog::DeploymentCatalog;
use betterlite_router::cooldown_decisions::policy::{CooldownPolicy, FailureThresholds};
use betterlite_router::cooldown_policy::counter::AllowedFailsStore;
use betterlite_router::cooldown_policy::error::CooldownPolicyError;
use betterlite_router::cooldown_policy::event::CooldownObserver;
use betterlite_router::cooldown_policy::service::{
    CooldownAttempt, CooldownPolicyService, CooldownSource,
};
use betterlite_router::cooldown_state::cooldown::CooldownCache;
use betterlite_router::cooldown_state::settings::Settings as CooldownSettings;
use betterlite_router::cooldown_state::shared::SharedCooldowns;
use betterlite_router::model_catalog_lookup::alias::AliasMap;
use betterlite_router::model_catalog_lookup::lookup::CatalogLookup;
use betterlite_router::routing_groups::index::RoutingGroupIndex;
use betterlite_router::routing_read_batch::{RequestReads, RoutingReadBatch};
use betterlite_router::routing_state::Settings as CounterSettings;
use betterlite_router::routing_state::counter::RouterCounters;
use betterlite_router::routing_state_store::RoutingStateStore;
use betterlite_router::routing_value_store::{Clock, RoutingValueStore, StoreSettings};

use super::fixtures::{FakeCooldowns, Recorder, seconds, spec};
use super::{ActiveOutput, Output, Payload};

type Built = (
    CooldownPolicyService<RoutingStateStore>,
    RouterCounters<RoutingStateStore>,
    Arc<Recorder>,
);

pub(super) async fn scenario(payload: &Payload) -> Result<Output, String> {
    let clock: Arc<Mutex<f64>> = Arc::new(Mutex::new(seconds(payload.now_millis)?));
    let handle = Arc::clone(&clock);
    let clock_fn: Clock = Arc::new(move || *handle.lock().unwrap_or_else(PoisonError::into_inner));
    let mut catalog = DeploymentCatalog::default();
    for deployment in &payload.deployments {
        catalog.push(spec(deployment)?);
    }
    let aliases = AliasMap::new();
    let groups = RoutingGroupIndex::default();
    let (service, minute_counters, recorder) = build_service(payload, &clock_fn)?;
    for _ in 0..payload.minute.successes {
        minute_counters
            .increment_successes_for_minute(&payload.target)
            .map_err(|error| error.to_string())?;
    }
    let mut cooled = Vec::new();
    for _ in 0..payload.failures {
        let attempt = CooldownAttempt {
            deployment_id: &payload.target,
            failure: &payload.failure,
            exception: "probe failure",
            header_retry_after: None,
            requested_model_group: None,
        };
        let result = cooling(&service, &catalog, &aliases, &groups, &attempt)
            .await
            .map_err(|error| error.to_string())?;
        cooled.push(result);
    }
    Ok(Output {
        cooled,
        events: recorder.events(),
        active: active_cooldowns(payload, &service).await?,
        failures_for_minute: minute_counters.failures_for_minute(&payload.target),
    })
}

// the lookup's dyn Fn provider is not Sync, so it stays inside this sync call and never reaches the future's await
fn cooling<'call>(
    service: &'call CooldownPolicyService<RoutingStateStore>,
    catalog: &DeploymentCatalog,
    aliases: &AliasMap,
    groups: &RoutingGroupIndex,
    attempt: &CooldownAttempt<'_>,
) -> impl Future<Output = Result<bool, CooldownPolicyError>> + Send + 'call {
    let no_provider = |_model: &str| -> Option<String> { None };
    let lookup = CatalogLookup {
        catalog,
        model_group_alias: aliases,
        routing_groups: groups,
        inferred_provider: &no_provider,
    };
    service.cool_deployment(&lookup, CooldownSource::Callback(attempt))
}

async fn active_cooldowns(
    payload: &Payload,
    service: &CooldownPolicyService<RoutingStateStore>,
) -> Result<Vec<ActiveOutput>, String> {
    let model_ids: Vec<String> = payload
        .deployments
        .iter()
        .map(|deployment| deployment.id.clone())
        .collect();
    let mut batch = RoutingReadBatch::default();
    let mut request = RequestReads::default();
    let entries = service
        .cooldown_debug_info(&mut batch, &mut request, &model_ids)
        .await
        .map_err(|error| error.to_string())?;
    Ok(entries
        .iter()
        .map(|cooldown| {
            let cooldown_time = cooldown.record.cooldown_time;
            ActiveOutput {
                model_id: cooldown.model_id.clone(),
                status_code: cooldown.record.status_code.clone(),
                cooldown_time: format!("{cooldown_time:.6}"),
            }
        })
        .collect())
}

fn build_service(payload: &Payload, clock_fn: &Clock) -> Result<Built, String> {
    let shared_cooldowns: Arc<dyn SharedCooldowns> = Arc::new(FakeCooldowns::default());
    let cooldowns = CooldownCache::new(
        &CooldownSettings::default(),
        shared_cooldowns,
        Arc::clone(clock_fn),
    );
    let local = Arc::new(
        RoutingValueStore::new(store_settings(), Arc::clone(clock_fn))
            .map_err(|error| error.to_string())?,
    );
    let counter_settings = CounterSettings {
        sync_interval: None,
        default_sync_interval: Duration::from_secs(60),
        should_batch_redis_writes: false,
    };
    let counters = RouterCounters::<RoutingStateStore>::new(
        counter_settings.clone(),
        Arc::clone(&local),
        None,
    );
    let minute_counters =
        RouterCounters::<RoutingStateStore>::new(counter_settings, Arc::clone(&local), None);
    let recorder = Arc::new(Recorder::default());
    let observer: Arc<dyn CooldownObserver> = Arc::<Recorder>::clone(&recorder);
    let service = CooldownPolicyService::new(
        cooldown_policy(payload),
        cooldowns,
        counters,
        AllowedFailsStore {
            local,
            remote: None,
        },
        vec![observer],
    );
    Ok((service, minute_counters, recorder))
}

fn cooldown_policy(payload: &Payload) -> CooldownPolicy {
    CooldownPolicy {
        allowed_fails: payload.router.allowed_fails,
        default_allowed_fails: payload.router.default_allowed_fails,
        cooldown_time: payload.router.cooldown_time,
        disable_cooldowns: payload.router.disable_cooldowns,
        allowed_fails_policy: None,
        thresholds: FailureThresholds::default(),
    }
}

const fn store_settings() -> StoreSettings {
    StoreSettings {
        max_size_in_memory: None,
        default_ttl_seconds: None,
        max_size_per_item: None,
        loaded_max_size_per_item_kib: 1024,
    }
}
