use std::future::Future;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_router::catalog_updates::catalog::DeploymentCatalog;
use betterlite_router::cooldown_policy::error::CooldownPolicyError;
use betterlite_router::cooldown_policy::service::{
    CooldownAttempt, CooldownPolicyService, CooldownSource,
};
use betterlite_router::model_catalog_lookup::alias::AliasMap;
use betterlite_router::model_catalog_lookup::lookup::CatalogLookup;
use betterlite_router::routing_groups::index::RoutingGroupIndex;
use betterlite_router::routing_read_batch::{RequestReads, RoutingReadBatch};
use betterlite_router::routing_state_store::RoutingStateStore;
use betterlite_router::routing_value_store::Clock;

use super::fixtures::{ProbeFixture, seconds, spec};
use super::{ActiveOutput, Output, Payload};

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
    let fixture = ProbeFixture::build(payload, &clock_fn)?;
    for _ in 0..payload.minute.successes {
        fixture
            .minute_counters
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
        let result = cooling(&fixture.service, &catalog, &aliases, &groups, &attempt)
            .await
            .map_err(|error| error.to_string())?;
        cooled.push(result);
    }
    Ok(Output {
        cooled,
        events: fixture.recorder.events(),
        active: active_cooldowns(payload, &fixture.service).await?,
        failures_for_minute: fixture.minute_counters.failures_for_minute(&payload.target),
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
    let mut model_ids = Vec::with_capacity(payload.deployments.len());
    for deployment in &payload.deployments {
        model_ids.push(deployment.id.clone());
    }
    let mut batch = RoutingReadBatch::default();
    let mut request = RequestReads::default();
    let entries = service
        .cooldown_debug_info(&mut batch, &mut request, &model_ids)
        .await
        .map_err(|error| error.to_string())?;
    let mut active = Vec::with_capacity(entries.len());
    for cooldown in &entries {
        let cooldown_time = cooldown.record.cooldown_time;
        active.push(ActiveOutput {
            model_id: cooldown.model_id.clone(),
            status_code: cooldown.record.status_code.clone(),
            cooldown_time: format!("{cooldown_time:.6}"),
        });
    }
    Ok(active)
}
