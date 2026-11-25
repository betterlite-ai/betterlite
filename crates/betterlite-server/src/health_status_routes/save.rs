use std::sync::Arc;

use betterlite_router::catalog_updates::catalog::DeploymentCatalog;
use betterlite_router::deployment_identity::spec::DeploymentSpec;
use chrono::{DateTime, Utc};

use crate::health_cycle::catalog::HealthCatalog;
use crate::health_cycle::cycle::{HealthCheckRequest, HealthCycleDeps, perform_health_check};
use crate::health_cycle::endpoints::HealthCheckResult;
use crate::health_persistence::{SingleHealthCheck, save_single};
use crate::health_probe_settings::model_info::CatalogModelInfo;
use crate::health_status_routes::error::HealthRouteError;
use crate::health_status_routes::state::{
    DetailVisibility, HealthCheckResults, HealthStatusState, SkipDisabledModels,
};

// _perform_health_check_and_save's arguments: one cycle's model list and target, plus who asked and when
#[derive(Clone, Copy)]
pub struct HealthProbeCall<'call> {
    pub model_list: &'call [DeploymentSpec],
    pub target_model: Option<&'call str>,
    pub cli_model: Option<&'call str>,
    pub model_id: Option<&'call str>,
    pub team_id: Option<&'call str>,
    pub user_id: Option<&'call str>,
    pub start_time: DateTime<Utc>,
}

pub async fn perform_health_check_and_save(
    state: &HealthStatusState,
    call: HealthProbeCall<'_>,
) -> Result<HealthCheckResults, HealthRouteError> {
    let result = run_cycle(state, call).await?;
    let results = health_results(result);
    schedule_save(state, call, &results);
    Ok(results)
}

fn health_results(result: HealthCheckResult) -> HealthCheckResults {
    HealthCheckResults {
        healthy_count: result.healthy_endpoints.len(),
        unhealthy_count: result.unhealthy_endpoints.len(),
        healthy_endpoints: result.healthy_endpoints,
        unhealthy_endpoints: result.unhealthy_endpoints,
    }
}

// The cycle with the injected transport, model facts, policy and tracker; the loaded catalog handle supplies the router argument
async fn run_cycle(
    state: &HealthStatusState,
    call: HealthProbeCall<'_>,
) -> Result<HealthCheckResult, HealthRouteError> {
    let loaded = state
        .cycle
        .catalog
        .as_ref()
        .map(|catalog| catalog.handle.load_full());
    let lookup = cycle_lookup(state, loaded.as_ref());
    let models = CatalogModelInfo {
        catalog: state.cycle.models.as_ref(),
    };
    let mut random = state.cycle.random.generator()?;
    let deps = HealthCycleDeps {
        provider: state.cycle.provider.as_ref(),
        models: &models,
        policy: &state.cycle.policy,
        random: &mut random,
        tasks: &state.tasks,
    };
    let request = cycle_request(state, call, lookup.as_ref());
    Ok(perform_health_check(request, deps).await?)
}

fn cycle_lookup<'state, 'loaded>(
    state: &'state HealthStatusState,
    loaded: Option<&'loaded Arc<DeploymentCatalog>>,
) -> Option<HealthCatalog<'loaded>>
where
    'state: 'loaded,
{
    let catalog = state.cycle.catalog.as_ref()?;
    Some(HealthCatalog {
        catalog: loaded?.as_ref(),
        model_group_alias: catalog.aliases.as_ref(),
        routing_groups: catalog.routing_groups.as_ref(),
        inferred_provider: catalog.inferred_provider.as_ref(),
    })
}

fn cycle_request<'call>(
    state: &HealthStatusState,
    call: HealthProbeCall<'call>,
    lookup: Option<&'call HealthCatalog<'call>>,
) -> HealthCheckRequest<'call> {
    HealthCheckRequest {
        model_list: call.model_list,
        model: call.target_model,
        model_id: call.model_id,
        team_id: call.team_id,
        cli_model: call.cli_model,
        details: state.health_check_details == DetailVisibility::Shown,
        max_concurrency: state
            .health_check_concurrency
            .and_then(|value| i64::try_from(value).ok()),
        skip_disabled: state.skip_disabled_models == SkipDisabledModels::Skip,
        lookup,
        instrumentation: None,
    }
}

// _save_health_check_to_db through the server's tracker: the response never waits for the write, and shutdown drains it
fn schedule_save(
    state: &HealthStatusState,
    call: HealthProbeCall<'_>,
    results: &HealthCheckResults,
) {
    let Some(rows) = state.rows.as_ref() else {
        return;
    };
    let Some(name) = call.cli_model.or(call.target_model) else {
        return;
    };
    let model_name = name.to_owned();
    let model_id = call.model_id.map(str::to_owned);
    let user_id = call.user_id.map(str::to_owned);
    let healthy = results.healthy_endpoints.clone();
    let unhealthy = results.unhealthy_endpoints.clone();
    let start_time = call.start_time;
    let store = Arc::clone(rows);
    let clock = Arc::clone(&state.clock);
    let _task = state.tasks.spawn(async move {
        let check = SingleHealthCheck {
            model_name: &model_name,
            model_id: model_id.as_deref(),
            healthy: &healthy,
            unhealthy: &unhealthy,
            start_time,
            user_id: user_id.as_deref(),
        };
        save_single(store.as_ref(), &check, clock.as_ref()).await;
    });
}
