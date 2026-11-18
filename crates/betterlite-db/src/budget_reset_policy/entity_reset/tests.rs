#![cfg(test)]

use super::{BudgetResetInput, ResetSchedule, reset_budget_values};
use crate::budget_reset_policy::rollover::RolloverPolicy;
use chrono::{DateTime, TimeZone, Utc};
use rust_decimal::Decimal;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Debug, thiserror::Error)]
#[error("the duration cannot be parsed")]
struct ScheduleFault;

#[derive(Clone)]
struct FixedSchedule {
    next: DateTime<Utc>,
    invocations: Arc<AtomicUsize>,
}

impl FixedSchedule {
    fn recorded_calls(&self) -> usize {
        self.invocations.load(Ordering::SeqCst)
    }
}

impl ResetSchedule for FixedSchedule {
    type Failure = ScheduleFault;

    fn next_reset_at(&self, budget_duration: &str) -> Result<DateTime<Utc>, ScheduleFault> {
        self.invocations.fetch_add(1, Ordering::SeqCst);
        if budget_duration == "unparseable" {
            return Err(ScheduleFault);
        }
        Ok(self.next)
    }
}

fn schedule() -> FixedSchedule {
    FixedSchedule {
        next: Utc.with_ymd_and_hms(2026, 2, 1, 0, 0, 0).unwrap(),
        invocations: Arc::new(AtomicUsize::new(0)),
    }
}

const fn input<'row>(
    spend: Option<Decimal>,
    max_budget: Option<Decimal>,
    budget_duration: Option<&'row str>,
    budget_reset_at: Option<DateTime<Utc>>,
) -> BudgetResetInput<'row> {
    BudgetResetInput {
        spend,
        max_budget,
        budget_duration,
        budget_reset_at,
    }
}

#[test]
fn entity_reset_zeroes_spend_without_rollover() {
    let schedule = schedule();
    let values = reset_budget_values(
        input(
            Some(Decimal::from(150_i32)),
            Some(Decimal::from(100_i32)),
            Some("1d"),
            None,
        ),
        RolloverPolicy::default(),
        &schedule,
    )
    .unwrap();
    assert_eq!(values.spend, Decimal::ZERO);
    assert_eq!(values.budget_reset_at, Some(schedule.next));
    assert_eq!(schedule.recorded_calls(), 1);
}

#[test]
fn entity_reset_carries_spend_with_rollover() {
    let schedule = schedule();
    let values = reset_budget_values(
        input(
            Some(Decimal::from(150_i32)),
            Some(Decimal::from(100_i32)),
            Some("1d"),
            None,
        ),
        RolloverPolicy { enabled: true },
        &schedule,
    )
    .unwrap();
    assert_eq!(values.spend, Decimal::from(50_i32));
    assert_eq!(values.budget_reset_at, Some(schedule.next));
}

#[test]
fn entity_reset_without_a_duration_keeps_the_reset_time() {
    let schedule = schedule();
    let existing = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
    let values = reset_budget_values(
        input(Some(Decimal::from(5_i32)), None, None, Some(existing)),
        RolloverPolicy::default(),
        &schedule,
    )
    .unwrap();
    assert_eq!(values.spend, Decimal::ZERO);
    assert_eq!(values.budget_reset_at, Some(existing));
    assert_eq!(schedule.recorded_calls(), 0);
}

#[test]
fn entity_reset_without_a_duration_keeps_a_missing_reset_time() {
    let schedule = schedule();
    let values = reset_budget_values(
        input(None, None, None, None),
        RolloverPolicy::default(),
        &schedule,
    )
    .unwrap();
    assert_eq!(values.spend, Decimal::ZERO);
    assert_eq!(values.budget_reset_at, None);
}

#[test]
fn entity_reset_advances_the_reset_time_from_the_schedule() {
    let schedule = FixedSchedule {
        next: Utc.with_ymd_and_hms(2026, 3, 1, 12, 30, 0).unwrap(),
        invocations: Arc::new(AtomicUsize::new(0)),
    };
    let values = reset_budget_values(
        input(
            None,
            None,
            Some("30d"),
            Some(Utc.with_ymd_and_hms(2025, 12, 1, 0, 0, 0).unwrap()),
        ),
        RolloverPolicy::default(),
        &schedule,
    )
    .unwrap();
    assert_eq!(
        values.budget_reset_at,
        Some(Utc.with_ymd_and_hms(2026, 3, 1, 12, 30, 0).unwrap())
    );
}

#[test]
fn entity_reset_propagates_a_schedule_failure() {
    let schedule = schedule();
    let result = reset_budget_values(
        input(Some(Decimal::from(5_i32)), None, Some("unparseable"), None),
        RolloverPolicy::default(),
        &schedule,
    );
    assert!(matches!(result, Err(ScheduleFault)));
}
