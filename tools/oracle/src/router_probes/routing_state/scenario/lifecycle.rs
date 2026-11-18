use betterlite_router::routing_state::metadata;
use betterlite_router::routing_value_store::RouteKey;
use rust_decimal::Decimal;
use rust_decimal::prelude::FromPrimitive as _;

use super::{Context, Observation, ScenarioError, ScenarioStep, required};

pub(super) async fn sync(
    index: usize,
    context: &Context<'_>,
) -> Result<Observation, ScenarioError> {
    let _synced = context.counters.sync_with_redis().await;
    Ok(Observation::Recorded { step: index })
}

pub(super) async fn retire(
    index: usize,
    context: &Context<'_>,
) -> Result<Observation, ScenarioError> {
    context.counters.retire().await;
    Ok(Observation::Recorded { step: index })
}

pub(super) fn switch(
    index: usize,
    context: &Context<'_>,
    down: bool,
) -> Result<Observation, ScenarioError> {
    context.server.set_down(down);
    Ok(Observation::Recorded { step: index })
}

pub(super) fn metadata(index: usize, step: &ScenarioStep) -> Result<Observation, ScenarioError> {
    let channels: metadata::CallbackChannels =
        serde_json::from_str(required(&step.channels, "a channels object")?)?;
    let channel = metadata::deployment_channel(&channels).map(|(name, _bucket)| name.to_owned());
    let tokens = metadata::get_counted_usage_tokens(&channels);
    Ok(Observation::Channel {
        step: index,
        channel,
        tokens,
    })
}

pub(super) async fn redis(
    index: usize,
    step: &ScenarioStep,
    context: &Context<'_>,
) -> Result<Observation, ScenarioError> {
    let key = RouteKey::parse(required(&step.key, "a key")?)
        .map_err(|source| ScenarioError::Key { source })?;
    let observed = context
        .store
        .increment_float(&key, 0.0_f64, None, false)
        .await?;
    let value = Decimal::from_f64(observed.value).ok_or(ScenarioError::NonFinite)?;
    Ok(Observation::Redis {
        step: index,
        value: value.normalize().to_string(),
    })
}
