use chrono::{DateTime, TimeDelta, Utc};

use super::ReloadSchedule;

mod tests;

#[must_use]
pub fn pod_reload_is_due(
    schedule: &ReloadSchedule,
    pod_applied_revision: u32,
    pod_data_loaded_at: DateTime<Utc>,
    current_time: DateTime<Utc>,
    description: &str,
) -> bool {
    if schedule.reload_revision != pod_applied_revision {
        tracing::info!("{description} reload triggered by manual reload request");
        return true;
    }
    let Some(interval_hours) = schedule.interval_hours else {
        return false;
    };
    if schedule.last_run_at.is_none() {
        tracing::info!("{description} reload triggered - schedule has never run");
        return true;
    }
    let interval = TimeDelta::try_hours(i64::from(interval_hours)).unwrap_or(TimeDelta::MAX);
    let elapsed = current_time.signed_duration_since(pod_data_loaded_at);
    if elapsed < interval {
        return false;
    }
    tracing::info!(
        seconds_since_data_loaded = elapsed.num_seconds(),
        interval_hours,
        "{description} reload triggered by interval"
    );
    true
}
