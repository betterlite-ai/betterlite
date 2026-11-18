use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use std::error::Error;

use super::rollover::{RolloverPolicy, reset_spend};

// B2317 owns the duration parser and the reset settings; the db crate cannot depend on it, so the phase supplies this port.
pub trait ResetSchedule {
    type Failure: Error + Send + Sync + 'static;

    fn next_reset_at(&self, budget_duration: &str) -> Result<DateTime<Utc>, Self::Failure>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BudgetResetInput<'row> {
    pub spend: Option<Decimal>,
    pub max_budget: Option<Decimal>,
    pub budget_duration: Option<&'row str>,
    pub budget_reset_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BudgetResetValues {
    pub spend: Decimal,
    pub budget_reset_at: Option<DateTime<Utc>>,
}

// _reset_budget_common: spend becomes the carried-or-zero value and budget_reset_at advances only when a duration is set; the key, user and team helpers all run this.
pub fn reset_budget_values<Schedule>(
    input: BudgetResetInput<'_>,
    policy: RolloverPolicy,
    schedule: &Schedule,
) -> Result<BudgetResetValues, Schedule::Failure>
where
    Schedule: ResetSchedule,
{
    let spend = reset_spend(policy, input.spend, input.max_budget);
    let Some(duration) = input.budget_duration else {
        return Ok(BudgetResetValues {
            spend,
            budget_reset_at: input.budget_reset_at,
        });
    };
    let next_reset_at = schedule.next_reset_at(duration)?;
    Ok(BudgetResetValues {
        spend,
        budget_reset_at: Some(next_reset_at),
    })
}

mod tests;
