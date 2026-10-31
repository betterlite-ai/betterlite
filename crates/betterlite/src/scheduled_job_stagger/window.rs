use super::{
    FALLBACK_STATS_JOB_ID, MONTHLY_SPEND_REPORT_JOB_ID, PTU_ROLLUP_JOB_ID,
    PTU_ROLLUP_LOCK_TTL_SECONDS,
};

#[must_use]
pub fn cron_dedupe_seconds(job_id: &str) -> Option<u32> {
    match job_id {
        MONTHLY_SPEND_REPORT_JOB_ID | FALLBACK_STATS_JOB_ID => Some(3600),
        PTU_ROLLUP_JOB_ID => Some(PTU_ROLLUP_LOCK_TTL_SECONDS),
        _ => None,
    }
}

#[must_use]
pub fn window_for(
    job_id: &str,
    period_seconds: Option<u32>,
    configured_window_seconds: u32,
) -> u32 {
    let mut window = configured_window_seconds;
    if let Some(period) = period_seconds {
        window = window.min(period);
    }
    if let Some(horizon) = cron_dedupe_seconds(job_id) {
        window = window.min(horizon);
    }
    window
}

mod tests;
