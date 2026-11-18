use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use rust_decimal::prelude::{FromPrimitive as _, ToPrimitive as _};

use betterlite_router::router_observations::usage::DeploymentUsage;
use betterlite_router::routing_value_store::RouteKey;
use betterlite_router::usage_v2_accounting::deployment::DeploymentFacts;
use betterlite_router::usage_v2_accounting::error::RpmLimitExceeded;
use betterlite_router::usage_v2_accounting::success::{
    HiddenParams, StandardLoggingObject, SuccessEvent,
};

use super::{
    Context, DeploymentInput, Observation, ScenarioError, ScenarioStep, StandardLoggingInput,
    StepKind,
};

pub(super) async fn apply(
    index: usize,
    step: &ScenarioStep,
    context: &Context<'_>,
) -> Result<Observation, ScenarioError> {
    match step.kind {
        StepKind::PreCall => {
            let deployment = step.deployment.as_ref().ok_or(ScenarioError::Missing {
                what: "a deployment",
            })?;
            pre_call(index, step, deployment, context).await
        }
        StepKind::Success => {
            let event = SuccessEvent {
                call_type: step.call_type.as_deref(),
                standard_logging_object: logged(step.standard_logging_object.as_ref()),
            };
            let now = at(step.now_millis)?;
            let _key = if step.sync {
                context.accounting.log_success_now(&event, now)
            } else {
                context.accounting.log_success(&event, now).await
            };
            Ok(Observation::Recorded { step: index })
        }
        StepKind::Sync => {
            let _pushed = context.accounting.counters.sync_with_redis().await;
            Ok(Observation::Recorded { step: index })
        }
        StepKind::SeedStore | StepKind::Store => store(index, step, context).await,
        StepKind::Down => Ok(switch(index, context, true)),
        StepKind::Up => Ok(switch(index, context, false)),
    }
}

// lowest_tpm_rpm_v2.pre_call_check / async_pre_call_check, picked by the step's sync flag
async fn pre_call(
    index: usize,
    step: &ScenarioStep,
    deployment: &DeploymentInput,
    context: &Context<'_>,
) -> Result<Observation, ScenarioError> {
    let facts = facts(deployment)?;
    let now = at(step.now_millis)?;
    let failure = if step.sync {
        context.accounting.pre_call_now(&facts, now).err()
    } else {
        context.accounting.pre_call(&facts, now).await.err()
    };
    Ok(failure.map_or_else(
        || Observation::Allowed { step: index },
        |error| rejected(index, &error),
    ))
}

// seed_store writes a shared counter directly; store reads one back without changing it
async fn store(
    index: usize,
    step: &ScenarioStep,
    context: &Context<'_>,
) -> Result<Observation, ScenarioError> {
    let key = parse_key(step)?;
    if step.kind == StepKind::SeedStore {
        let value = step
            .value
            .ok_or(ScenarioError::Missing { what: "a value" })?;
        seed(context, &key, value).await?;
        return Ok(Observation::Recorded { step: index });
    }
    let total = read_total(context, &key).await?;
    let value = Decimal::from_f64(total).ok_or(ScenarioError::NonFinite)?;
    Ok(Observation::Store {
        step: index,
        value: value.normalize().to_string(),
    })
}

// seed_store adds the step's value to the shared counter
async fn seed(context: &Context<'_>, key: &RouteKey, value: i64) -> Result<(), ScenarioError> {
    let amount = Decimal::from(value)
        .to_f64()
        .ok_or(ScenarioError::NonFinite)?;
    let _observed = context
        .store
        .increment_float(key, amount, None, false)
        .await?;
    Ok(())
}

// store reads the shared counter without changing it
async fn read_total(context: &Context<'_>, key: &RouteKey) -> Result<f64, ScenarioError> {
    let observed = context
        .store
        .increment_float(key, 0.0_f64, None, false)
        .await?;
    Ok(observed.value)
}

fn switch(index: usize, context: &Context<'_>, down: bool) -> Observation {
    context.server.set_down(down);
    Observation::Recorded { step: index }
}

fn facts(deployment: &DeploymentInput) -> Result<DeploymentFacts<'_>, ScenarioError> {
    let model_id = deployment
        .model_id
        .as_deref()
        .ok_or(ScenarioError::Missing {
            what: "a deployment id",
        })?;
    let model = deployment
        .model
        .as_deref()
        .ok_or(ScenarioError::Missing { what: "a model" })?;
    Ok(DeploymentFacts {
        model_id,
        model,
        limits: DeploymentUsage {
            tpm: None,
            rpm: deployment.rpm,
            params_tpm: None,
            params_rpm: deployment.params_rpm,
            model_info_tpm: None,
            model_info_rpm: deployment.model_info_rpm,
            io_token_limits: false,
        },
        num_retries: deployment.num_retries,
    })
}

fn logged(logged: Option<&StandardLoggingInput>) -> Option<StandardLoggingObject<'_>> {
    logged.map(|logged| StandardLoggingObject {
        model_group: logged.model_group.as_deref(),
        hidden_params: HiddenParams {
            litellm_model_name: logged.model.as_deref(),
        },
        model_id: logged.model_id.as_deref(),
        total_tokens: logged.total_tokens,
    })
}

fn rejected(index: usize, error: &RpmLimitExceeded) -> Observation {
    Observation::Rejected {
        step: index,
        status_code: RpmLimitExceeded::STATUS_CODE,
        code: RpmLimitExceeded::CODE.to_owned(),
        error_type: RpmLimitExceeded::ERROR_TYPE.to_owned(),
        retry_after: error.retry_after_seconds.map(|seconds| seconds.to_string()),
        num_retries: error.num_retries,
    }
}

fn parse_key(step: &ScenarioStep) -> Result<RouteKey, ScenarioError> {
    let text = step
        .key
        .as_deref()
        .ok_or(ScenarioError::Missing { what: "a key" })?;
    RouteKey::parse(text).map_err(|source| ScenarioError::Key { source })
}

fn at(millis: u64) -> Result<DateTime<Utc>, ScenarioError> {
    i64::try_from(millis)
        .ok()
        .and_then(DateTime::<Utc>::from_timestamp_millis)
        .ok_or(ScenarioError::Time { millis })
}
