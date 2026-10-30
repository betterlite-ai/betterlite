use super::{JobSnapshot, window::cron_dedupe_seconds};

#[must_use]
pub fn is_staggerable(job: &JobSnapshot) -> bool {
    if job.next_run_time.is_some() {
        return false;
    }
    if job.period_seconds.is_some() {
        return true;
    }
    cron_dedupe_seconds(&job.id).is_some()
}

mod tests;
