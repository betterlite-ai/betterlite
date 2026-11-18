use std::time::Duration;

use rust_decimal::Decimal;

use super::{Context, Observation, ScenarioError, ScenarioStep, TTL_SECONDS, required};

pub(super) async fn pre_call(
    index: usize,
    step: &ScenarioStep,
    context: &Context<'_>,
) -> Result<Observation, ScenarioError> {
    let key = window_key(step)?;
    let ttl = Duration::from_secs(TTL_SECONDS);
    for _run in 0..step.repeat.unwrap_or(1) {
        context
            .counters
            .increment_window_value(&key, Decimal::ONE, ttl)
            .await?;
    }
    Ok(Observation::Recorded { step: index })
}

// lowest_tpm_rpm_v2.async_pre_call_check's window key: "{model_id}:{deployment_name}:rpm:{hour}-{minute}"
fn window_key(step: &ScenarioStep) -> Result<String, ScenarioError> {
    let deployment = required(&step.deployment_id, "a deployment id")?;
    let model = required(&step.model, "a model")?;
    let minutes = step.now_millis / 60_000;
    let hour = (minutes / 60) % 24;
    let minute = minutes % 60;
    Ok(format!("{deployment}:{model}:rpm:{hour:02}-{minute:02}"))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Outcome {
    Success,
    Failure,
}

// track_deployment_metrics.increment_deployment_successes_for_current_minute / failures
pub(super) fn outcome(
    index: usize,
    step: &ScenarioStep,
    context: &Context<'_>,
    kind: Outcome,
) -> Result<Observation, ScenarioError> {
    let deployment = required(&step.deployment_id, "a deployment id")?;
    match kind {
        Outcome::Success => {
            let key = context
                .counters
                .increment_successes_for_minute(deployment)?;
            Ok(Observation::Key { step: index, key })
        }
        Outcome::Failure => {
            context.counters.increment_failures_for_minute(deployment)?;
            Ok(Observation::Recorded { step: index })
        }
    }
}

pub(super) fn tracked(index: usize, context: &Context<'_>) -> Result<Observation, ScenarioError> {
    let keys = context.counters.take_tracked_keys();
    Ok(Observation::Keys {
        step: index,
        keys: keys.into_iter().collect(),
    })
}
