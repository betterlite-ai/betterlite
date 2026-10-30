use chrono::Utc;

use super::{JobScheduler, JobSubmission, SubmissionListener};

pub fn attach_job_timing_logger(scheduler: &mut dyn JobScheduler) {
    let listener: SubmissionListener = Box::new(log_job_submitted);
    scheduler.add_submission_listener(listener);
}

fn log_job_submitted(event: &dyn JobSubmission) {
    let Some(scheduled) = event.scheduled_run_times().first() else {
        return;
    };
    let started = Utc::now();
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
