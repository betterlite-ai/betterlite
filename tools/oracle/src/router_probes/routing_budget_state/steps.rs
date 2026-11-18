use std::str::FromStr as _;
use std::sync::{Arc, PoisonError};

use betterlite_context::local_ttl_cache::CacheValue;
use betterlite_router::routing_budget_state::event::SuccessEvent;
use rust_decimal::Decimal;

use super::scenario::{Observation, Run, ScenarioError, ScenarioStep, StepKind, money};
use super::store::render;

const SPEND_FIELD: &str = "spend";

pub(super) async fn apply(
    index: usize,
    step: &ScenarioStep,
    run: &mut Run,
) -> Result<Observation, ScenarioError> {
    match step.kind {
        StepKind::Success => success(index, step, run).await,
        StepKind::Sync => {
            let _synced = run.state.sync_with_redis().await;
            Ok(recorded(index, step.kind))
        }
        StepKind::BeginSync => begin_sync(index, run).await,
        StepKind::EndSync => end_sync(index, run).await,
        StepKind::Shutdown => {
            run.state.shutdown().await;
            Ok(recorded(index, step.kind))
        }
        StepKind::Memory => memory(index, step, run),
        StepKind::Redis => redis(index, step, run),
        StepKind::FailWrites => {
            run.store.set_fail(true);
            Ok(recorded(index, step.kind))
        }
        StepKind::SucceedWrites => {
            run.store.set_fail(false);
            Ok(recorded(index, step.kind))
        }
        StepKind::HoldWrites => {
            run.store.set_hold(true);
            Ok(recorded(index, step.kind))
        }
        StepKind::ReleaseWrites => {
            run.store.release_writes();
            Ok(recorded(index, step.kind))
        }
    }
}

fn recorded(step: usize, kind: StepKind) -> Observation {
    Observation {
        kind,
        step,
        value: None,
    }
}

// async_log_success_event: the payload's provider, model id and tags name the windows the cost lands in
async fn success(
    index: usize,
    step: &ScenarioStep,
    run: &Run,
) -> Result<Observation, ScenarioError> {
    let cost = money(step.cost.as_deref())?;
    let tags = step.tags.clone().unwrap_or_default();
    let event = SuccessEvent {
        call_type: step.call_type.as_deref(),
        response_cost: cost,
        model_id: step.model_id.as_deref(),
        custom_llm_provider: step.provider.as_deref(),
        tags: &tags,
    };
    run.state.log_success(&event).await?;
    Ok(recorded(index, step.kind))
}

// one sync while the store is held, so a success can land after the flush detached its batch
async fn begin_sync(index: usize, run: &mut Run) -> Result<Observation, ScenarioError> {
    if !run.store.holding() {
        return Err(ScenarioError::NotHeld);
    }
    let state = Arc::clone(&run.state);
    let _task = run
        .syncs
        .spawn(async move { state.sync_with_redis().await });
    run.store.entered().notified().await;
    Ok(recorded(index, StepKind::BeginSync))
}

async fn end_sync(index: usize, run: &mut Run) -> Result<Observation, ScenarioError> {
    let Some(synced) = run.syncs.join_next().await else {
        return Err(ScenarioError::BeyondSync);
    };
    synced.map_err(|source| ScenarioError::Join { source })?;
    Ok(recorded(index, StepKind::EndSync))
}

fn memory(index: usize, step: &ScenarioStep, run: &Run) -> Result<Observation, ScenarioError> {
    let key = step
        .key
        .as_deref()
        .ok_or(ScenarioError::Missing { what: "a key" })?;
    let value = run
        .memory
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(key)?;
    let spend = value
        .as_ref()
        .and_then(memory_spend)
        .map(|spend| spend.normalize().to_string());
    Ok(Observation {
        kind: step.kind,
        step: index,
        value: spend,
    })
}

fn redis(index: usize, step: &ScenarioStep, run: &Run) -> Result<Observation, ScenarioError> {
    let key = step
        .key
        .as_deref()
        .ok_or(ScenarioError::Missing { what: "a key" })?;
    let spend = run.store.spend(key).map(render);
    Ok(Observation {
        kind: step.kind,
        step: index,
        value: spend,
    })
}

fn memory_spend(value: &CacheValue) -> Option<Decimal> {
    let CacheValue::Object(fields) = value else {
        return None;
    };
    let CacheValue::Text(text) = fields.get(SPEND_FIELD)? else {
        return None;
    };
    Decimal::from_str(text).ok()
}
