use std::future::Future;

use chrono::{NaiveDateTime, Timelike as _, Utc};
use rust_decimal::Decimal;

use crate::window_queue::stamp::{WindowStamp, decode_window_stamp};

use super::error::WindowSpendError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowSeedTotals {
    pub total: Decimal,
    pub before_batch: Decimal,
}

// _WINDOW_SPEND_ENTITY_TYPES: only a key or team has spend log rows a window seed can sum
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeedEntity {
    Key,
    Team,
}

impl SeedEntity {
    #[must_use]
    pub fn from_litellm_name(name: &str) -> Option<Self> {
        match name {
            "key" => Some(Self::Key),
            "team" => Some(Self::Team),
            _unknown => None,
        }
    }
}

pub struct SeedQuery<'query> {
    pub entity: SeedEntity,
    pub entity_id: &'query str,
    pub window_start: NaiveDateTime,
    pub batch_started_at: Option<NaiveDateTime>,
}

pub trait WindowSpendLogs: Send + Sync {
    fn seed_totals(
        &self,
        query: &SeedQuery<'_>,
    ) -> impl Future<Output = Result<WindowSeedTotals, WindowSpendError>> + Send;
}

// The queue stores a window stamp as naive UTC, so a naive value is already the database wall clock and an offset one converts to it
pub fn window_instant(text: &str) -> Result<NaiveDateTime, WindowSpendError> {
    let stamp = decode_window_stamp(text.as_bytes())?;
    Ok(match stamp {
        WindowStamp::Naive(value) => value,
        WindowStamp::Aware(value) => value.with_timezone(&Utc).naive_utc(),
    })
}

// _exclusion_upper_bound: LiteLLM_SpendLogs.startTime is TIMESTAMP(3), so a millisecond rounding of the batch's own first row must not slip under the cutoff
#[must_use]
pub fn exclusion_upper_bound(started_at: NaiveDateTime) -> NaiveDateTime {
    started_at.with_nanosecond(0).unwrap_or(started_at)
}

// _seed_base: the full sum minus this batch keeps every other request, and the sum before the batch floors it when this batch's log rows have not landed yet
pub fn seed_base(
    totals: &WindowSeedTotals,
    batch_spend: Decimal,
) -> Result<Decimal, WindowSpendError> {
    let discounted = totals
        .total
        .checked_sub(batch_spend)
        .ok_or(WindowSpendError::Overflow)?;
    Ok(discounted.max(totals.before_batch))
}
