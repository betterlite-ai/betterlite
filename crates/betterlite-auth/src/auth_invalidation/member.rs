use rust_decimal::{Decimal, prelude::ToPrimitive as _};
use tokio::task::try_id;
use tracing::warn;

use super::error::InvalidationError;
use super::keys::InvalidationKey;
use super::registry::MembershipInflight;
use super::{AuthInvalidation, SPEND_COUNTER_TTL_SECONDS, SPEND_DB_FLOOR_TTL_SECONDS};

pub(super) async fn wait_for_membership_load<Load>(
    inflight: &MembershipInflight<Load>,
    reservation_key: &str,
) where
    Load: Send + Sync,
{
    let Some(mut load) = inflight.pop(reservation_key) else {
        return;
    };
    if Some(load.id) == try_id() {
        return;
    }
    let _finished = load.completion.changed().await;
}

pub(super) async fn reset_spend_counters(
    invalidation: &AuthInvalidation,
    user_id: &str,
    team_id: &str,
    spend: Decimal,
) -> Result<(), InvalidationError> {
    let counter_key = InvalidationKey::SpendCounter { user_id, team_id }.render();
    let floor_key = InvalidationKey::SpendFloor { user_id, team_id }.render();
    let amount = spend
        .to_f64()
        .ok_or(InvalidationError::SpendOutOfRange(spend))?;
    invalidation
        .spend
        .set_memory(&counter_key, amount, SPEND_COUNTER_TTL_SECONDS)
        .map_err(|source| InvalidationError::Memory(Box::new(source)))?;
    set_counter_redis(invalidation, &counter_key, amount).await?;
    invalidation
        .spend
        .set_memory(&floor_key, amount, SPEND_DB_FLOOR_TTL_SECONDS)
        .map_err(|source| InvalidationError::Memory(Box::new(source)))?;
    invalidation
        .transport
        .publish(&counter_key, Some(amount), Some(SPEND_COUNTER_TTL_SECONDS))
        .await;
    invalidation
        .transport
        .publish(&floor_key, Some(amount), Some(SPEND_DB_FLOOR_TTL_SECONDS))
        .await;
    Ok(())
}

async fn set_counter_redis(
    invalidation: &AuthInvalidation,
    counter_key: &str,
    amount: f64,
) -> Result<(), InvalidationError> {
    let Err(error) = invalidation
        .spend
        .set_redis(counter_key, amount, SPEND_COUNTER_TTL_SECONDS)
        .await
    else {
        return Ok(());
    };
    warn!(
        subject = counter_key,
        error = %error,
        "Failed to set spend counter {counter_key} in Redis after reset: {error}; deleting it instead so the next read reseeds from the DB rather than keeping the stale pre-reset value authoritative"
    );
    let Err(delete_error) = invalidation.spend.delete_redis(counter_key).await else {
        return Ok(());
    };
    warn!(
        subject = counter_key,
        error = %delete_error,
        "Failed to delete stale spend counter {counter_key} in Redis after a failed reset write"
    );
    Err(InvalidationError::SpendResetUnavailable(Box::new(error)))
}
