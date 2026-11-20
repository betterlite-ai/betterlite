use std::time::Instant;

use betterlite_config::sections::health_probe_policy::Settings as ProbePolicy;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_router::deployment_identity::spec::DeploymentSpec;
use rand::Rng;

// ast-grep-ignore: no-long-names - LiteLLM's _dependency_deployments_to_probe, kept for parity
use crate::health::router::dependency_deployments_to_probe as dependencies_to_probe;
use crate::health::router::error::RouterHealthError;
use crate::health_cycle::catalog::HealthCatalog;
use crate::health_cycle::dispatch::{DispatchSettings, perform_health_checks};
use crate::health_cycle::endpoints::HealthCheckResult;
use crate::health_cycle::error::HealthCycleError;
use crate::health_cycle::ports::ProviderHealth;
use crate::health_cycle::probe::{PreparedProbe, prepare_probe};
use crate::health_cycle::telemetry::{
    CycleCounts, InstrumentationContext, cycle_complete, cycle_failed, cycle_skipped, cycle_start,
};
use crate::health_cycle::verdict::rebuild_after_verdicts;
use crate::health_probe_settings::model_info::HealthCheckModelInfo;
use crate::health_probe_settings::targeting::{
    filter_deployments_by_id, health_check_eligible, narrow_to_target,
};

// perform_health_check's call: the deployment list and every way LiteLLM narrows it, plus the display and logging switches
pub struct HealthCheckRequest<'call> {
    pub model_list: &'call [DeploymentSpec],
    pub model: Option<&'call str>,
    pub model_id: Option<&'call str>,
    pub team_id: Option<&'call str>,
    pub cli_model: Option<&'call str>,
    pub details: bool,
    pub max_concurrency: Option<i64>,
    pub skip_disabled: bool,
    pub lookup: Option<&'call HealthCatalog<'call>>,
    pub instrumentation: Option<&'call InstrumentationContext>,
}

// The cycle's dependencies, injected: the transport, the model facts, the probe policy and the random source
pub struct HealthCycleDeps<'deps, R, M>
where
    R: Rng + Send,
    M: HealthCheckModelInfo + Sync,
{
    pub provider: &'deps dyn ProviderHealth,
    pub models: &'deps M,
    pub policy: &'deps ProbePolicy,
    pub random: &'deps mut R,
}

struct CyclePlan {
    checked: Vec<DeploymentSpec>,
    dependency_probes: Vec<DeploymentSpec>,
    counts: CycleCounts,
    started: Instant,
}

// The CLI fallback row carries no model_info, so nothing invents an id for it and the id dedupe skips it
fn synthetic_deployment(cli_model: &str) -> DeploymentSpec {
    let mut litellm_params = JsonObject::new();
    litellm_params.insert("model".to_owned(), Json::Text(cli_model.to_owned()));
    DeploymentSpec {
        model_name: cli_model.to_owned(),
        litellm_params,
        model_info: JsonObject::new(),
        extra: JsonObject::new(),
    }
}

fn starting_models(
    model_list: &[DeploymentSpec],
    cli_model: Option<&str>,
    context: &InstrumentationContext,
) -> Option<Vec<DeploymentSpec>> {
    if !model_list.is_empty() {
        return Some(model_list.to_vec());
    }
    match cli_model {
        Some(cli_model) => Some(vec![synthetic_deployment(cli_model)]),
        None => {
            cycle_skipped(context, "no_models");
            None
        }
    }
}

// _dependency_deployments_to_probe over the same eligibility filter, so an opted-out deployment cannot re-enter as a dependency
fn expand_dependencies(
    request: &HealthCheckRequest<'_>,
    model_list: &[DeploymentSpec],
    requested: &[DeploymentSpec],
) -> Result<Vec<DeploymentSpec>, RouterHealthError> {
    let Some(catalog) = request.lookup else {
        return Ok(Vec::new());
    };
    let pool = health_check_eligible(model_list, request.skip_disabled);
    catalog.with_lookup(|lookup| dependencies_to_probe(requested, &pool, &lookup))
}

// perform_health_check up to the dispatch: narrow, filter, dedupe, expand dependencies, log the start line
fn plan_cycle(
    model_list: &[DeploymentSpec],
    request: &HealthCheckRequest<'_>,
    context: &InstrumentationContext,
) -> Result<Option<CyclePlan>, HealthCycleError> {
    let started = Instant::now();
    let requested_count = model_list.len();
    let narrowed = health_check_eligible(
        &narrow_to_target(model_list, request.model, request.model_id, request.team_id),
        request.skip_disabled,
    );
    if narrowed.is_empty() {
        cycle_skipped(context, "no_models_after_filter");
        return Ok(None);
    }
    let requested = filter_deployments_by_id(&narrowed);
    let counts = CycleCounts {
        requested: requested_count,
        post_filter: narrowed.len(),
        deduped: requested.len(),
    };
    let dependency_probes = expand_dependencies(request, model_list, &requested)?;
    let checked: Vec<DeploymentSpec> = requested
        .iter()
        .chain(dependency_probes.iter())
        .cloned()
        .collect();
    cycle_start(context, &counts, request.max_concurrency);
    Ok(Some(CyclePlan {
        checked,
        dependency_probes,
        counts,
        started,
    }))
}

fn prepare_cycle<M, R>(
    checked: &[DeploymentSpec],
    models: &M,
    policy: &ProbePolicy,
    random: &mut R,
) -> Vec<PreparedProbe>
where
    M: HealthCheckModelInfo,
    R: Rng,
{
    checked
        .iter()
        .map(|deployment| prepare_probe(deployment, models, policy, random))
        .collect()
}

async fn run_cycle<R, M>(
    checked: &[DeploymentSpec],
    dependency_probes: &[DeploymentSpec],
    request: &HealthCheckRequest<'_>,
    deps: &mut HealthCycleDeps<'_, R, M>,
    context: &InstrumentationContext,
) -> Result<HealthCheckResult, HealthCycleError>
where
    R: Rng + Send,
    M: HealthCheckModelInfo + Sync,
{
    let prepared = prepare_cycle(checked, deps.models, deps.policy, deps.random);
    let settings = DispatchSettings {
        provider: deps.provider,
        details: request.details,
        max_concurrency: request.max_concurrency,
        instrumentation: context,
    };
    let probed = perform_health_checks(checked, &prepared, &settings).await;
    rebuild_after_verdicts(probed, checked, request.lookup, dependency_probes)
}

// The cycle's closing line, on the result or on the error: both carry the model count and the run's duration
pub(crate) fn finish_cycle(
    outcome: Result<HealthCheckResult, HealthCycleError>,
    context: &InstrumentationContext,
    model_count: usize,
    started: Instant,
) -> Result<HealthCheckResult, HealthCycleError> {
    let elapsed = started.elapsed();
    match outcome {
        Ok(result) => {
            cycle_complete(
                context,
                model_count,
                result.healthy_endpoints.len(),
                result.unhealthy_endpoints.len(),
                elapsed,
            );
            Ok(result)
        }
        Err(error) => {
            cycle_failed(context, model_count, elapsed, &error.to_string());
            Err(error)
        }
    }
}

pub async fn perform_health_check<R, M>(
    request: HealthCheckRequest<'_>,
    mut deps: HealthCycleDeps<'_, R, M>,
) -> Result<HealthCheckResult, HealthCycleError>
where
    R: Rng + Send,
    M: HealthCheckModelInfo + Sync,
{
    let context = request.instrumentation.cloned().unwrap_or_default();
    let Some(model_list) = starting_models(request.model_list, request.cli_model, &context) else {
        return Ok(HealthCheckResult::default());
    };
    let Some(plan) = plan_cycle(&model_list, &request, &context)? else {
        return Ok(HealthCheckResult::default());
    };
    let result = run_cycle(
        &plan.checked,
        &plan.dependency_probes,
        &request,
        &mut deps,
        &context,
    )
    .await;
    finish_cycle(result, &context, plan.counts.deduped, plan.started)
}
