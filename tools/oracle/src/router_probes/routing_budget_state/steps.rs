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
    let spend = match step.kind {
        StepKind::Success => success(step, run).await?,
        StepKind::Sync => {
            let _synced = run.state.sync_with_redis().await;
            None
        }
        StepKind::BeginSync => begin_sync(run).await?,
        StepKind::EndSync => end_sync(run).await?,
        StepKind::Shutdown => {
            run.state.shutdown().await?;
            None
        }
        StepKind::Memory => memory_value(required_key(step)?, run)?,
        StepKind::Redis => redis_value(required_key(step)?, run),
        StepKind::FailWrites => {
            run.store.fail.store(true, Ordering::SeqCst);
            None
        }
        StepKind::SucceedWrites => {
            run.store.fail.store(false, Ordering::SeqCst);
            None
        }
        StepKind::HoldWrites => {
            run.store.hold.store(true, Ordering::SeqCst);
            None
        }
        StepKind::ReleaseWrites => {
            // one permit, so a release that lands before the write waits for it is not lost
            run.store.release.notify_one();
            None
        }
    };
    Ok(observed(index, step, spend))
}

// async_log_success_event: the payload's provider, model id and tags name the windows the cost lands in
async fn success(step: &ScenarioStep, run: &Run) -> Result<Option<Decimal>, ScenarioError> {
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
    Ok(None)
}

// one sync while the store is held, so a success can land after the flush detached its batch
async fn begin_sync(run: &mut Run) -> Result<Option<Decimal>, ScenarioError> {
    if !run.store.hold.load(Ordering::SeqCst) {
        return Err(ScenarioError::NotHeld);
    }
    let state = Arc::clone(&run.state);
    let _task = run
        .syncs
        .spawn(async move { state.sync_with_redis().await });
    run.store.entered.notified().await;
    Ok(None)
}

async fn end_sync(run: &mut Run) -> Result<Option<Decimal>, ScenarioError> {
    let Some(synced) = run.syncs.join_next().await else {
        return Err(ScenarioError::BeyondSync);
    };
    synced.map_err(|source| ScenarioError::Join { source })?;
    Ok(None)
}

fn required_key(step: &ScenarioStep) -> Result<&str, ScenarioError> {
    step.key
        .as_deref()
        .ok_or(ScenarioError::Missing { what: "a key" })
}

fn memory_value(key: &str, run: &Run) -> Result<Option<Decimal>, ScenarioError> {
    let value = run
        .memory
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(key)?;
    Ok(value.as_ref().and_then(memory_spend))
}

fn redis_value(key: &str, run: &Run) -> Option<Decimal> {
    let spends = run
        .store
        .spends
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    spends.get(key).copied()
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
