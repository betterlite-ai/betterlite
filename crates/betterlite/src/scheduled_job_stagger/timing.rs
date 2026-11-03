use chrono::{DateTime, Utc};

use super::JobSubmission;

pub fn log_job_submitted(event: &dyn JobSubmission, clock: &dyn Fn() -> DateTime<Utc>) {
    let Some(scheduled) = event.scheduled_run_times().first() else {
        return;
    };
    let started = clock();
    let delay = started.signed_duration_since(*scheduled);
    tracing::debug!(
        job_id = event.job_id(),
        scheduled_run_time = %scheduled.to_rfc3339(),
        actual_start_time = %started.to_rfc3339(),
        delay_milliseconds = delay.num_milliseconds(),
        "scheduled job started"
    );
}

mod tests;
