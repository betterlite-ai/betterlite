use super::ReloadSchedule;
use super::ReloadScheduleStatus;
use super::next_run_at::next_run_at;

mod tests;

#[must_use]
pub fn reload_schedule_status(schedule: Option<&ReloadSchedule>) -> ReloadScheduleStatus {
    let Some(schedule) = schedule else {
        return ReloadScheduleStatus {
            scheduled: false,
            interval_hours: None,
            last_run: None,
            next_run: None,
        };
    };
    let next_run = next_run_at(schedule);
    ReloadScheduleStatus {
        scheduled: schedule.interval_hours.is_some(),
        interval_hours: schedule.interval_hours,
        last_run: schedule.last_run_at.map(|instant| instant.to_rfc3339()),
        next_run: next_run.map(|instant| instant.to_rfc3339()),
    }
}
