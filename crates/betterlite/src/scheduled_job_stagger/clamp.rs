use super::window::cron_dedupe_seconds;

// An override past the dedup horizon would leave two replicas running the same cron job at once.
#[must_use]
pub fn clamped_override(job_id: &str, requested: u32) -> u32 {
    let Some(horizon) = cron_dedupe_seconds(job_id) else {
        return requested;
    };
    if requested < horizon {
        return requested;
    }
    let replacement = horizon.saturating_sub(1);
    tracing::warn!(
        job_id,
        requested,
        horizon,
        replacement,
        "scheduled job stagger override clamped: it would separate replicas by more than the job's dedup horizon"
    );
    replacement
}

mod tests;
