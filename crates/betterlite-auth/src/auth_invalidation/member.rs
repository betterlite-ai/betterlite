use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;
use tracing::warn;

use super::error::InvalidationError;
use super::keys::InvalidationKey;
use super::registry::MembershipInflight;
use super::{AuthInvalidation, SPEND_COUNTER_TTL_SECONDS, SPEND_DB_FLOOR_TTL_SECONDS};
use crate::auth_cache::keys::CacheKey;

impl AuthInvalidation {
    pub async fn invalidate_team_member_spend<Load>(
        &self,
        inflight: &MembershipInflight<Load>,
        user_id: &str,
        team_id: &str,
        new_spend: Option<Decimal>,
    ) -> Result<(), InvalidationError>
    where
        Load: Send + Sync,
    {
        let reservation_key = CacheKey::TeamMembershipReservation { user_id, team_id }.render();
        self.wait_for_membership_load(inflight, &reservation_key)
            .await;
        if let Some(spend) = new_spend {
            self.reset_spend_counters(user_id, team_id, spend).await?;
        }
        let membership_keys = [
            CacheKey::TeamMembershipAuth { team_id, user_id }.render(),
            reservation_key,
        ];
        self.transport
            .evict_and_broadcast(&membership_keys, self)
            .await;
        Ok(())
    }

    async fn wait_for_membership_load<Load>(
        &self,
        inflight: &MembershipInflight<Load>,
        reservation_key: &str,
    ) where
        Load: Send + Sync,
    {
        let Some(mut load) = inflight.pop(reservation_key) else {
            return;
        };
        if Some(load.id) == tokio::task::try_id() {
            return;
        }
        let _finished = load.completion.changed().await;
    }

    async fn reset_spend_counters(
        &self,
        user_id: &str,
        team_id: &str,
        spend: Decimal,
    ) -> Result<(), InvalidationError> {
        let counter_key = InvalidationKey::SpendCounter { user_id, team_id }.render();
        let floor_key = InvalidationKey::SpendFloor { user_id, team_id }.render();
        let amount = spend
            .to_f64()
            .ok_or(InvalidationError::SpendOutOfRange(spend))?;
        self.spend
            .set_memory(&counter_key, amount, SPEND_COUNTER_TTL_SECONDS)
            .map_err(|source| InvalidationError::Memory(Box::new(source)))?;
        self.set_counter_redis(&counter_key, amount).await?;
        self.spend
            .set_memory(&floor_key, amount, SPEND_DB_FLOOR_TTL_SECONDS)
            .map_err(|source| InvalidationError::Memory(Box::new(source)))?;
        self.transport
            .publish(&counter_key, Some(amount), Some(SPEND_COUNTER_TTL_SECONDS))
            .await;
        self.transport
            .publish(&floor_key, Some(amount), Some(SPEND_DB_FLOOR_TTL_SECONDS))
            .await;
        Ok(())
    }

    async fn set_counter_redis(
        &self,
        counter_key: &str,
        amount: f64,
    ) -> Result<(), InvalidationError> {
        let Err(error) = self
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
        let Err(delete_error) = self.spend.delete_redis(counter_key).await else {
            return Ok(());
        };
        warn!(
            subject = counter_key,
            error = %delete_error,
            "Failed to delete stale spend counter {counter_key} in Redis after a failed reset write"
        );
        Err(InvalidationError::SpendResetUnavailable(Box::new(error)))
    }
}
