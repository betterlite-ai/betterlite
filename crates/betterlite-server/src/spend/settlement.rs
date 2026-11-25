pub mod error;
pub mod ports;

mod tests;

use std::error::Error as StdError;

use betterlite_auth::reservation_settlement::adjust::AdjustmentMode;
use betterlite_auth::reservation_settlement::handle::{
    ReservationHandle, get_reserved_counter_keys,
};
use betterlite_auth::reservation_settlement::reconcile::ReservationFinality;
use betterlite_auth::spend_counter_batch::keys::{
    CounterId, EntityCounterIds, PostCallCounterKeys, post_call_counter_keys,
};
use betterlite_auth::spend_increments::SpendIncrementInput;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_redis::request_batch::lock;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;

use crate::spend_cache_update::arm_cache_read;
use crate::spend_cache_update::cache::SpendCache;

use self::error::SpendSettlementError;
use self::ports::{CounterBatchScope, ReservationSettlement, SpendCounterIncrement, SpendWrite};

// the request facts _update_database_and_spend_counters forwards; the cost callback found the reservation before this runs
pub struct SettlementInput<'input> {
    pub reservation: Option<&'input ReservationHandle>,
    pub response_cost: Decimal,
    pub token: Option<&'input str>,
    pub user_id: Option<&'input str>,
    pub end_user_id: Option<&'input str>,
    pub team_id: Option<&'input str>,
    pub org_id: Option<&'input str>,
    pub project_id: Option<&'input str>,
    pub start_time: Option<DateTime<Utc>>,
    pub tags: Option<&'input [JsonData]>,
    pub model_access_groups: Option<&'input [JsonData]>,
    pub update_cache_read_keys: &'input [String],
}

// the reservation settlement, the request's counter batch, the update_cache tier and increment_spend_counters
pub struct SettlementPorts<'ports, Counters> {
    pub reservations: &'ports dyn ReservationSettlement,
    pub batch: &'ports dyn CounterBatchScope,
    pub cache: &'ports dyn SpendCache,
    pub counters: &'ports Counters,
}

// _update_database_and_spend_counters: the reconcile reads on its own, then one counter batch spans the write and the increments
pub async fn settle_spend_write<Write, Counters>(
    input: &SettlementInput<'_>,
    write: &Write,
    ports: &SettlementPorts<'_, Counters>,
) -> Result<bool, SpendSettlementError<Write::Error>>
where
    Write: SpendWrite,
    Counters: SpendCounterIncrement,
{
    if let Some(reservation) = input.reservation {
        reconcile_before_write(ports.reservations, reservation, input.response_cost).await;
    }
    let mut counter_keys = input
        .reservation
        .map(get_reserved_counter_keys)
        .unwrap_or_default();
    counter_keys.extend(post_call_counter_keys(&PostCallCounterKeys {
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
    }));
    let _batch = ports.batch.open(&counter_keys);
    write_then_count(input, write, ports).await
}

// _update_database_and_spend_counters_in_batch: nothing touches the counters until the write has answered
async fn write_then_count<Write, Counters>(
    input: &SettlementInput<'_>,
    write: &Write,
    ports: &SettlementPorts<'_, Counters>,
) -> Result<bool, SpendSettlementError<Write::Error>>
where
    Write: SpendWrite,
    Counters: SpendCounterIncrement,
{
    let charged = match write.update_database().await {
        Ok(charged) => charged,
        Err(error) => {
            if let Some(reservation) = input.reservation {
                release_after_failed_write(ports.reservations, reservation).await;
            }
            return Err(SpendSettlementError::Write(error));
        }
    };
    if !charged {
        if let Some(reservation) = input.reservation {
            ports
                .reservations
                .release(reservation)
                .await
                .map_err(SpendSettlementError::Release)?;
        }
        return Ok(false);
    }
    count_charged(input, ports).await
}

// the row is already persisted, so a counter failure only drops the reserved counters and never retries the write
async fn count_charged<WriteError, Counters>(
    input: &SettlementInput<'_>,
    ports: &SettlementPorts<'_, Counters>,
) -> Result<bool, SpendSettlementError<WriteError>>
where
    WriteError: StdError + Send + Sync + 'static,
    Counters: SpendCounterIncrement,
{
    arm_cache_read(input.update_cache_read_keys, ports.cache)
        .map_err(SpendSettlementError::Prefetch)?;
    let increment = SpendIncrementInput {
        token: input.token,
        team_id: input.team_id,
        user_id: input.user_id,
        response_cost: Some(input.response_cost),
        org_id: input.org_id,
        budget_reservation: input.reservation,
        end_user_id: input.end_user_id,
        tags: input.tags,
        request_started_at: input.start_time,
        model_access_groups: input.model_access_groups,
        project_id: input.project_id,
    };
    let Err(error) = ports.counters.increment(&increment).await else {
        return Ok(true);
    };
    if let Some(reservation) = input.reservation {
        drop_reserved_counters(
            ports.reservations,
            reservation,
            "Failed to invalidate budget reservation counters after spend counter update failed",
        )
        .await;
    }
    Err(SpendSettlementError::Counters(error))
}

// _reconcile_budget_reservation_before_db_update: a failed reconcile must not block the spend write
async fn reconcile_before_write(
    reservations: &dyn ReservationSettlement,
    reservation: &ReservationHandle,
    actual_cost: Decimal,
) {
    let reconciled = reservations
        .reconcile(
            reservation,
            actual_cost,
            ReservationFinality::LeaveOpen,
            AdjustmentMode::Defer,
        )
        .await;
    let Err(error) = reconciled else {
        return;
    };
    tracing::warn!(
        %error,
        "Failed to reconcile budget reservation before persisting spend; invalidating reserved counters"
    );
    drop_reserved_counters(
        reservations,
        reservation,
        "Failed to invalidate budget reservation counters after pre-persist reconcile failed",
    )
    .await;
}

// the finalized stamp holds even when the invalidation fails, so nothing reprocesses the reservation
async fn drop_reserved_counters(
    reservations: &dyn ReservationSettlement,
    reservation: &ReservationHandle,
    failure: &str,
) {
    if let Err(error) = reservations.invalidate(reservation).await {
        tracing::error!(%error, "{failure}");
    }
    lock(&reservation.0).finalized = true;
}

// a failed write still frees the reservation, and a failed release drops its counters; the write error is what the caller sees
async fn release_after_failed_write(
    reservations: &dyn ReservationSettlement,
    reservation: &ReservationHandle,
) {
    let Err(error) = reservations.release(reservation).await else {
        return;
    };
    tracing::error!(
        %error,
        "Failed to release budget reservation after database update failed"
    );
    if let Err(invalidation) = reservations.invalidate(reservation).await {
        tracing::error!(
            error = %invalidation,
            "Failed to invalidate budget reservation counters after release failed"
        );
    }
}
