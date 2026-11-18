use chrono::{DateTime, TimeDelta, Utc};

use super::ReloadSchedule;

mod tests;

#[must_use]
pub fn next_run_at(schedule: &ReloadSchedule) -> Option<DateTime<Utc>> {
    let interval_hours = schedule.interval_hours?;
    let last_run_at = schedule.last_run_at?;
    let interval = TimeDelta::try_hours(i64::from(interval_hours))?;
    last_run_at.checked_add_signed(interval)
}
