use super::{JobSnapshot, NextRunTime, window::cron_dedupe_seconds};

#[must_use]
pub fn is_staggerable(job: &JobSnapshot) -> bool {
    if matches!(job.next_run_time, NextRunTime::Explicit(_)) {
        return false;
    }
    if job.period_seconds.is_some() {
        return true;
    }
    cron_dedupe_seconds(&job.id).is_some()
}

mod tests;
