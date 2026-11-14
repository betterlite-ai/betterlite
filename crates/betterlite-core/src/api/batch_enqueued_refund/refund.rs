use std::sync::PoisonError;

use crate::api::batch_enqueued_reserve::ReservationBackend;
use crate::api::batch_enqueued_reserve::keys::COUNTER_KEY_PREFIX;

use super::record::ReservationRecord;
use super::store::{EnqueuedLocalStore, ReservationRecordScripts};
use super::{BatchEnqueuedRefundError, EnqueuedTokenRefund};

pub(crate) async fn refund<Scripts, Store>(
    service: &EnqueuedTokenRefund<Scripts, Store>,
    reservation: &ReservationRecord,
) -> Result<(), BatchEnqueuedRefundError>
where
    Scripts: ReservationRecordScripts,
    Store: EnqueuedLocalStore,
{
    if reservation.tokens <= 0 || reservation.scopes.is_empty() {
        return Ok(());
    }
    if reservation.backend == ReservationBackend::Redis {
        return refund_redis(service, reservation).await;
    }
    if reservation.owner != service.owner {
        tracing::warn!(
            "Skipping enqueued-token refund granted in another worker's memory; its counters expire with the TTL"
        );
        return Ok(());
    }
    let mut store = service.store.lock().unwrap_or_else(PoisonError::into_inner);
    for scope in &reservation.scopes {
        let counter_key = format!("{COUNTER_KEY_PREFIX}{}:{}", scope.key.as_str(), scope.value);
        let current = store.counter(&counter_key)?;
        match current.checked_sub(reservation.tokens) {
            Some(remaining) if remaining > 0 => {
                store.set_counter(&counter_key, remaining, service.settings.ttl_seconds)?;
            }
            _ => store.delete(&counter_key),
        }
    }
    drop(store);
    Ok(())
}

async fn refund_redis<Scripts, Store>(
    service: &EnqueuedTokenRefund<Scripts, Store>,
    reservation: &ReservationRecord,
) -> Result<(), BatchEnqueuedRefundError>
where
    Scripts: ReservationRecordScripts,
    Store: EnqueuedLocalStore,
{
    let Some(scripts) = &service.scripts else {
        tracing::warn!(
            "No Redis client for a Redis-granted enqueued-token refund; leaked increments expire with the TTL"
        );
        return Ok(());
    };
    for scope in &reservation.scopes {
        let counter_key = format!("{COUNTER_KEY_PREFIX}{}:{}", scope.key.as_str(), scope.value);
        if let Err(error) = scripts.refund(&counter_key, reservation.tokens).await {
            tracing::warn!(
                error = %error,
                "Redis enqueued-token refund failed; leaked increments expire with the TTL"
            );
            return Ok(());
        }
    }
    Ok(())
}
