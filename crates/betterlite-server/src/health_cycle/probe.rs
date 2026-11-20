use std::time::Duration;

use betterlite_config::sections::health_probe_policy::Settings as ProbePolicy;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_router::deployment_identity::spec::DeploymentSpec;
use rand::Rng;

use crate::health_cycle::ports::{ProbeFailure, ProbeRequest, ProbeResponse, ProviderHealth};
use crate::health_probe_settings::mode::resolve_health_check_mode;
use crate::health_probe_settings::model_info::HealthCheckModelInfo;
use crate::health_probe_settings::prepare::update_params_for_health_check;
use crate::health_probe_settings::timeout::{HealthCheckTimeout, run_with_timeout};
use crate::router_dependencies::classify_strategy_router_model;

const TIMEOUT_FIELD: &str = "health_check_timeout";
const TIMEOUT_MESSAGE: &str = "Timeout exceeded";
const TIMEOUT_STATUS: u16 = 408;
// _run_model_health_check always sends this input, independent of the env-overridable prompt
const HEALTH_CHECK_INPUT: &str = "test from litellm";

// _run_model_health_check's answer before the provider is reached: a strategy router returns {}, which reads as healthy
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PreparedProbe {
    Skipped,
    Ready {
        request: ProbeRequest,
        timeout: Duration,
    },
    Failed(ProbeFailure),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProbeOutcome {
    Healthy(ProbeResponse),
    // _run_model_health_check returned its {error, exception} dict, which the row shows as its error text
    Failed(ProbeFailure),
    // the probe raised before producing a dict, so the row keeps only its params beside the status
    Raised(ProbeFailure),
}

// _is_strategy_router_deployment: only a text litellm_params.model can name one of the reserved router names
fn is_strategy_router(deployment: &DeploymentSpec) -> bool {
    match deployment.litellm_params.get("model") {
        Some(Json::Text(model)) => classify_strategy_router_model(model).is_some(),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))
        | None => false,
    }
}

fn invalid_timeout() -> ProbeFailure {
    ProbeFailure::internal(format!(
        "model_info.{TIMEOUT_FIELD} must be a number of seconds"
    ))
}

// model_info.get("health_check_timeout") or HEALTH_CHECK_TIMEOUT_SECONDS: zero is falsy and keeps the default, a negative value times out at once
fn probe_timeout(model_info: &JsonObject, fallback: Duration) -> Result<Duration, ProbeFailure> {
    let Some(value) = model_info.get(TIMEOUT_FIELD) else {
        return Ok(fallback);
    };
    if matches!(*value, Json::Null) {
        return Ok(fallback);
    }
    let Json::Number(number) = value else {
        return Err(invalid_timeout());
    };
    let Some(seconds) = number.as_f64() else {
        return Err(invalid_timeout());
    };
    if seconds > 0.0_f64 {
        return Ok(Duration::try_from_secs_f64(seconds).unwrap_or(Duration::MAX));
    }
    if seconds < 0.0_f64 {
        return Ok(Duration::ZERO);
    }
    Ok(fallback)
}

fn ready_probe<M, R>(
    deployment: &DeploymentSpec,
    models: &M,
    policy: &ProbePolicy,
    random: &mut R,
) -> Result<PreparedProbe, ProbeFailure>
where
    M: HealthCheckModelInfo,
    R: Rng,
{
    let mode =
        resolve_health_check_mode(&deployment.model_info, &deployment.litellm_params, models);
    let params = match update_params_for_health_check(
        &deployment.model_info,
        &deployment.litellm_params,
        policy,
        models,
        random,
    ) {
        Ok(params) => params,
        Err(error) => return Err(ProbeFailure::internal(error.to_string())),
    };
    let timeout = probe_timeout(&deployment.model_info, policy.timeout)?;
    let request = ProbeRequest {
        params,
        mode,
        prompt: policy.prompt.clone(),
        input: vec![HEALTH_CHECK_INPUT.to_owned()],
    };
    Ok(PreparedProbe::Ready { request, timeout })
}

// _run_model_health_check: resolve the mode, prepare the params, then time the one provider call this deployment gets
#[must_use]
pub fn prepare_probe<M, R>(
    deployment: &DeploymentSpec,
    models: &M,
    policy: &ProbePolicy,
    random: &mut R,
) -> PreparedProbe
where
    M: HealthCheckModelInfo,
    R: Rng,
{
    if is_strategy_router(deployment) {
        return PreparedProbe::Skipped;
    }
    match ready_probe(deployment, models, policy, random) {
        Ok(prepared) => prepared,
        Err(failure) => PreparedProbe::Failed(failure),
    }
}

fn timeout_failure() -> ProbeFailure {
    ProbeFailure {
        message: TIMEOUT_MESSAGE.to_owned(),
        status_code: TIMEOUT_STATUS,
        raw_request_typed_dict: None,
    }
}

// run_with_timeout cancels the awaited probe alone, so a timeout leaves every sibling future running
#[must_use]
pub async fn execute_probe(
    provider: &dyn ProviderHealth,
    prepared: &PreparedProbe,
) -> ProbeOutcome {
    match prepared {
        PreparedProbe::Skipped => ProbeOutcome::Healthy(ProbeResponse::default()),
        PreparedProbe::Failed(failure) => ProbeOutcome::Raised(failure.clone()),
        PreparedProbe::Ready { request, timeout } => {
            let probe = provider.check(request.clone());
            match run_with_timeout(probe, *timeout).await {
                Ok(Ok(response)) => ProbeOutcome::Healthy(response),
                Ok(Err(failure)) => ProbeOutcome::Failed(failure),
                Err(HealthCheckTimeout) => ProbeOutcome::Failed(timeout_failure()),
            }
        }
    }
}
