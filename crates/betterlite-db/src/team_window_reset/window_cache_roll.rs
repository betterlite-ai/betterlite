use std::error::Error;

use chrono::{DateTime, TimeDelta, Utc};

use crate::budget_permission_rows::JsonData;
use crate::budget_reset_policy::entity_reset::ResetSchedule;
use crate::budget_reset_policy::rollover::RolloverPolicy;
use crate::window_spend::store::{WindowRoll, WindowSpendStore};

use super::cache::WindowCounterCache;
use super::carry_over::window_carried_spend;
use super::error::WindowResetError;
use super::json_persist::window_fields;
use super::reset_at::advance_window_reset_at;
use super::source::WindowSource;
use super::window_expiry::window_is_expired;

// B199's duration_in_seconds: the db crate cannot depend on betterlite-core, so the composition root supplies the parser and its settings.
pub trait WindowDurationSeconds {
    type Failure: Error + Send + Sync + 'static;

    fn duration_seconds(&self, budget_duration: &str) -> Result<i64, Self::Failure>;
}

pub struct WindowResetter<'ports, Schedule, Cache, Durations> {
    pub store: &'ports WindowSpendStore,
    pub rollover: RolloverPolicy,
    pub schedule: &'ports Schedule,
    pub cache: &'ports Cache,
    pub durations: &'ports Durations,
    pub now: DateTime<Utc>,
}

impl<Schedule, Cache, Durations> WindowResetter<'_, Schedule, Cache, Durations>
where
    Schedule: ResetSchedule,
    Cache: WindowCounterCache,
    Durations: WindowDurationSeconds,
{
    // _reset_expired_window: only an expired window moves; its counter settles first, then the JSON expiry advances, then the maintained row roll is best effort.
    pub(crate) async fn reset_expired_window(
        &self,
        source: WindowSource,
        row_id: &str,
        window: &mut JsonData,
    ) -> Result<bool, WindowResetError<Schedule::Failure>> {
        let fields = window_fields(row_id, window)?;
        let expired =
            window_is_expired(fields.reset_at, self.now.naive_utc()).map_err(|source| {
                WindowResetError::Expiry {
                    row: row_id.to_owned(),
                    source,
                }
            })?;
        if !expired {
            return Ok(false);
        }
        let duration = fields.budget_duration.to_owned();
        let counter_key = source.window_counter_key(row_id, &duration);
        let carried =
            window_carried_spend(fields.max_budget, &counter_key, self.rollover, self.cache).await;
        self.cache.set_memory(&counter_key, carried);
        if let Err(error) = self.cache.set_redis(&counter_key, carried).await {
            tracing::warn!(
                counter = %counter_key,
                error = %error,
                "Failed to reset the window spend counter in Redis; the in-memory counter is already at the carried value"
            );
        }
        let next = advance_window_reset_at(row_id, window, &duration, self.schedule)?;
        self.roll(source, row_id, &duration, next).await;
        Ok(true)
    }

    // _roll_window_spend_row: a duration the parser refuses and a failed update each only lose the maintained total, which the read path rebuilds from the logs.
    async fn roll(&self, source: WindowSource, row_id: &str, duration: &str, next: DateTime<Utc>) {
        let offset = match self.durations.duration_seconds(duration) {
            Ok(seconds) => TimeDelta::try_seconds(seconds),
            Err(error) => {
                tracing::warn!(
                    duration = %duration,
                    error = %error,
                    "Unparsable budget_duration; leaving the window spend row as it is"
                );
                return;
            }
        };
        let Some(new_window_start) =
            offset.and_then(|step| next.naive_utc().checked_sub_signed(step))
        else {
            tracing::warn!(
                duration = %duration,
                "The window start before the next reset overflows the calendar; leaving the window spend row as it is"
            );
            return;
        };
        let roll = WindowRoll {
            entity_type: source.entity_type(),
            entity_id: row_id,
            window_duration: duration,
            new_window_start,
            now: self.now,
        };
        if let Err(error) = self.store.roll_window_spend_row(&roll).await {
            tracing::warn!(
                entity = %row_id,
                window = %duration,
                error = %error,
                "Failed to roll the budget window spend row; the counter reset still stands"
            );
        }
    }
}

mod tests;
