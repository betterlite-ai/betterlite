use std::str::FromStr as _;
use std::sync::atomic::Ordering;
use std::sync::{Arc, PoisonError};

use betterlite_context::local_ttl_cache::CacheValue;
use betterlite_router::routing_budget_state::event::{CallType, SuccessEvent};
use rust_decimal::Decimal;

use super::scenario::{Observation, Run, ScenarioError, ScenarioStep, StepKind, money_text};

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
            Ok(observed(index, step, None))
        }
        StepKind::BeginSync => begin_sync(index, step, run).await,
        StepKind::EndSync => end_sync(index, step, run).await,
        StepKind::Shutdown => {
            run.state.shutdown().await?;
            Ok(observed(index, step, None))
        }
        StepKind::Memory => Ok(observed(index, step, memory_value(step, run)?)),
        StepKind::Redis => Ok(observed(index, step, redis_value(step, run)?)),
        StepKind::FailWrites => {
            run.store.fail.store(true, Ordering::SeqCst);
            Ok(observed(index, step, None))
        }
        StepKind::SucceedWrites => {
            run.store.fail.store(false, Ordering::SeqCst);
            Ok(observed(index, step, None))
        }
        StepKind::HoldWrites => {
            run.store.hold.store(true, Ordering::SeqCst);
            Ok(observed(index, step, None))
        }
        StepKind::ReleaseWrites => {
            // one permit, so a release that lands before the write waits for it is not lost
            run.store.release.notify_one();
            Ok(observed(index, step, None))
        }
    }
}

// async_log_success_event: the payload's provider, model id and tags name the windows the cost lands in
async fn success(
    index: usize,
    step: &ScenarioStep,
    run: &Run,
) -> Result<Observation, ScenarioError> {
    let cost = money_text(
        step.cost
            .as_deref()
            .ok_or(ScenarioError::Missing { what: "a cost" })?,
    )?;
    let tags = step.tags.clone().unwrap_or_default();
    let event = SuccessEvent {
        call_type: CallType::parse(step.call_type.as_deref()),
        response_cost: cost,
        model_id: step.model_id.as_deref(),
        custom_llm_provider: step.provider.as_deref(),
        tags: &tags,
    };
    run.state.log_success(&event).await?;
    Ok(observed(index, step, None))
}

// one sync while the store is held, so a success can land after the flush detached its batch
async fn begin_sync(
    index: usize,
    step: &ScenarioStep,
    run: &mut Run,
) -> Result<Observation, ScenarioError> {
    if !run.store.hold.load(Ordering::SeqCst) {
        return Err(ScenarioError::NotHeld);
    }
    let state = Arc::clone(&run.state);
    let _task = run
        .syncs
        .spawn(async move { state.sync_with_redis().await });
    run.store.entered.notified().await;
    Ok(observed(index, step, None))
}

async fn end_sync(
    index: usize,
    step: &ScenarioStep,
    run: &mut Run,
) -> Result<Observation, ScenarioError> {
    let Some(synced) = run.syncs.join_next().await else {
        return Err(ScenarioError::BeyondSync);
    };
    synced.map_err(|source| ScenarioError::Join { source })?;
    Ok(observed(index, step, None))
}

fn memory_value(step: &ScenarioStep, run: &Run) -> Result<Option<Decimal>, ScenarioError> {
    let key = step
        .key
        .as_deref()
        .ok_or(ScenarioError::Missing { what: "a key" })?;
    let value = run
        .memory
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(key)?;
    Ok(value.as_ref().and_then(memory_spend))
}

fn redis_value(step: &ScenarioStep, run: &Run) -> Result<Option<Decimal>, ScenarioError> {
    let key = step
        .key
        .as_deref()
        .ok_or(ScenarioError::Missing { what: "a key" })?;
    let spends = run
        .store
        .spends
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    Ok(spends.get(key).copied())
}

// one answer per step: the kind it was asked for, its index, and the spend it made observable
fn observed(index: usize, step: &ScenarioStep, spend: Option<Decimal>) -> Observation {
    Observation {
        kind: step.kind,
        step: index,
        value: spend.map(|spend| spend.normalize().to_string()),
    }
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
