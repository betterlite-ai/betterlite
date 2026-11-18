use std::time::Duration;

use super::{Context, Observation, ScenarioStep, TTL_SECONDS, required};

pub(super) async fn pre_call(
    index: usize,
    step: &ScenarioStep,
    context: &Context<'_>,
) -> Result<Observation, String> {
    let key = window_key(step)?;
    let ttl = Duration::from_secs(TTL_SECONDS);
    for _run in 0..step.repeat.unwrap_or(1) {
        let _amount = context
            .counters
            .increment_window_value(&key, 1, ttl)
            .await
            .map_err(|error| error.to_string())?;
    }
    Ok(Observation::Recorded { step: index })
}

// lowest_tpm_rpm_v2.async_pre_call_check's window key: "{model_id}:{deployment_name}:rpm:{hour}-{minute}"
fn window_key(step: &ScenarioStep) -> Result<String, String> {
    let deployment = required(&step.deployment_id, "a deployment id")?;
    let model = required(&step.model, "a model")?;
    let minutes = step.now_millis / 60_000;
    let hour = (minutes / 60) % 24;
    let minute = minutes % 60;
    Ok(format!("{deployment}:{model}:rpm:{hour:02}-{minute:02}"))
}

pub(super) fn success(
    index: usize,
    step: &ScenarioStep,
    context: &Context<'_>,
) -> Result<Observation, String> {
    let deployment = required(&step.deployment_id, "a deployment id")?;
    let key = context
        .counters
        .increment_successes_for_minute(deployment)
        .map_err(|error| error.to_string())?;
    Ok(Observation::Key { step: index, key })
}

pub(super) fn failure(
    index: usize,
    step: &ScenarioStep,
    context: &Context<'_>,
) -> Result<Observation, String> {
    let deployment = required(&step.deployment_id, "a deployment id")?;
    context
        .counters
        .increment_failures_for_minute(deployment)
        .map_err(|error| error.to_string())?;
    Ok(Observation::Recorded { step: index })
}

pub(super) fn outcomes(
    index: usize,
    step: &ScenarioStep,
    context: &Context<'_>,
) -> Result<Observation, String> {
    let deployment = required(&step.deployment_id, "a deployment id")?;
    let successes = context.counters.successes_for_minute(deployment);
    let failures = context.counters.failures_for_minute(deployment);
    Ok(Observation::Outcomes {
        step: index,
        successes,
        failures,
    })
}

pub(super) fn tracked(index: usize, context: &Context<'_>) -> Result<Observation, String> {
    let keys = context.counters.take_tracked_keys();
    Ok(Observation::Keys {
        step: index,
        keys: keys.into_iter().collect(),
    })
}
