pub mod error;
mod key_team;
mod reservation;
mod scopes;
mod sources;
pub mod window;

mod tests;

use std::sync::Arc;

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::spend_rows::rows::SpendCounterRows as DbSpendCounterRows;
use betterlite_redis::redis_cache::RedisCache;
use betterlite_redis::request_batch::lock;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;

use crate::reservation_settlement::counters::SettlementCounters;
use crate::reservation_settlement::handle::{ReservationHandle, stamp_reservation_actual_cost};
use crate::spend_counter_batch::batch::PendingSpendIncrement;
use crate::spend_counter_batch::keys::{
    CounterId, EntityCounterIds, PostCallCounterKeys, post_call_counter_keys,
};
use crate::spend_counter_batch::scope::SpendCounterScope;
use crate::spend_reseed::cache::dual::ReseedCacheError;
use crate::spend_reseed::cache::{ReseedCache, WindowReseedCache};
use crate::spend_reseed::rows::SpendCounterRows;
use crate::spend_writes::SpendCounterWrites;
use crate::spend_writes::apply::apply_spend_counter_increments;
use crate::spend_writes::layers::CounterLayers;

use self::error::SpendIncrementError;
use self::reservation::{ReservationCounterUpdate, reconcile_reservation};
use self::scopes::{ScopeCharge, run_scopes};
use self::window::WindowSpendQueue;

// the request facts increment_spend_counters charges, one field per LiteLLM parameter
pub struct SpendIncrementInput<'input> {
    pub token: Option<&'input str>,
    pub team_id: Option<&'input str>,
    pub user_id: Option<&'input str>,
    pub response_cost: Option<Decimal>,
    pub org_id: Option<&'input str>,
    pub budget_reservation: Option<&'input ReservationHandle>,
    pub end_user_id: Option<&'input str>,
    pub tags: Option<&'input [JsonData]>,
    pub request_started_at: Option<DateTime<Utc>>,
    pub model_access_groups: Option<&'input [JsonData]>,
    pub project_id: Option<&'input str>,
}

// the spend counter writes, batch scope and window queue the post-call fan-out reaches, injected so the gateway and the tests share one path
pub struct SpendIncrementServices<'services, Counters, Layers, Rows>
where
    Counters: SettlementCounters,
    Layers: ReseedCache<Error = ReseedCacheError> + CounterLayers + WindowReseedCache + 'static,
    Rows: SpendCounterRows + DbSpendCounterRows,
{
    pub writes: &'services SpendCounterWrites<Layers, Rows>,
    pub scope: &'services SpendCounterScope,
    pub redis: Option<&'services Arc<RedisCache>>,
    pub window_queue: &'services dyn WindowSpendQueue,
    pub counters: &'services Counters,
}

// increment_spend_counters: the batch scope opens over the post-call keys, so the reconcile and every warm check share one MGET
pub async fn increment_spend_counters<Counters, Layers, Rows>(
    input: &SpendIncrementInput<'_>,
    services: &SpendIncrementServices<'_, Counters, Layers, Rows>,
) -> Result<(), SpendIncrementError>
where
    Counters: SettlementCounters,
    Layers: ReseedCache<Error = ReseedCacheError> + CounterLayers + WindowReseedCache + 'static,
    Rows: SpendCounterRows + DbSpendCounterRows,
{
    let counter_keys = post_call_counter_keys(&PostCallCounterKeys {
        entity: EntityCounterIds {
            token: CounterId::from_text(input.token),
            team_id: CounterId::from_text(input.team_id),
            user_id: CounterId::from_text(input.user_id),
            org_id: CounterId::from_text(input.org_id),
            project_id: CounterId::from_text(input.project_id),
            end_user_id: CounterId::from_text(input.end_user_id),
        },
        tags: input.tags.unwrap_or_default(),
        model_access_groups: input.model_access_groups.unwrap_or_default(),
    });
    let _batch = services.scope.enter(services.redis, &counter_keys);
    increment_batched(input, services).await
}

// _increment_spend_counters_batched: the reconcile adjustments and every scope's increments go out in one apply, and only a clean run finalizes
async fn increment_batched<Counters, Layers, Rows>(
    input: &SpendIncrementInput<'_>,
    services: &SpendIncrementServices<'_, Counters, Layers, Rows>,
) -> Result<(), SpendIncrementError>
where
    Counters: SettlementCounters,
    Layers: ReseedCache<Error = ReseedCacheError> + CounterLayers + WindowReseedCache + 'static,
    Rows: SpendCounterRows + DbSpendCounterRows,
{
    let update = reconcile_reservation(
        services.counters,
        input.budget_reservation,
        input.response_cost,
    )
    .await;
    let Some(cost) = input.response_cost.filter(|amount| !amount.is_zero()) else {
        return apply_zero_cost(input.budget_reservation, services.writes, &update.pending).await;
    };
    let ReservationCounterUpdate {
        reserved_counter_keys,
        pending: reservation_pending,
    } = update;
    let charge = ScopeCharge {
        cost,
        reserved: reserved_counter_keys,
        request_started_at: input.request_started_at,
    };
    let outcome = run_scopes(input, &charge, services).await;
    let mut pending = reservation_pending;
    pending.extend(outcome.pending);
    apply_spend_counter_increments(services.writes, &pending).await?;
    if let Some(handle) = input.budget_reservation {
        stamp_reservation_actual_cost(handle, cost)?;
    }
    if let Some(error) = outcome.error {
        return Err(error);
    }
    if let Some(handle) = input.budget_reservation {
        lock(&handle.0).finalized = true;
    }
    Ok(())
}

// the zero-cost arm: the reservation still settles its adjustments and finalizes, while no scope counter is written
async fn apply_zero_cost<Layers, Rows>(
    handle: Option<&ReservationHandle>,
    writes: &SpendCounterWrites<Layers, Rows>,
    pending: &[PendingSpendIncrement],
) -> Result<(), SpendIncrementError>
where
    Layers: ReseedCache<Error = ReseedCacheError> + CounterLayers + 'static,
    Rows: SpendCounterRows,
{
    apply_spend_counter_increments(writes, pending).await?;
    let Some(reservation) = handle else {
        return Ok(());
    };
    stamp_reservation_actual_cost(reservation, Decimal::ZERO)?;
    lock(&reservation.0).finalized = true;
    Ok(())
}
