use chrono::{DateTime, NaiveDate, TimeDelta, Utc};

use super::ReloadSchedule;

mod tests;

#[must_use]
pub fn next_run_at(schedule: &ReloadSchedule) -> Option<DateTime<Utc>> {
    let interval_hours = schedule.interval_hours?;
    let last_run_at = schedule.last_run_at?;
    let interval = TimeDelta::try_hours(i64::from(interval_hours))?;
    let next_run = last_run_at.checked_add_signed(interval)?;
    // Python's datetime.max, where LiteLLM's timedelta addition raises OverflowError
    let last_python_microsecond = NaiveDate::from_ymd_opt(9999, 12, 31)?
        .and_hms_micro_opt(23, 59, 59, 999_999)?
        .and_utc();
    if next_run > last_python_microsecond {
        return None;
    }
    Some(next_run)
}
