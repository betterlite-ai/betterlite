#![cfg(test)]

use super::{cron_dedupe_seconds, window_for};
use crate::scheduled_job_stagger::{
    FALLBACK_STATS_JOB_ID, MONTHLY_SPEND_REPORT_JOB_ID, PTU_ROLLUP_JOB_ID,
};

#[test]
fn unknown_job_has_no_dedupe_horizon() {
    assert_eq!(cron_dedupe_seconds("some_other_job"), None);
}

#[test]
fn known_cron_jobs_have_their_own_horizon() {
    assert_eq!(cron_dedupe_seconds(MONTHLY_SPEND_REPORT_JOB_ID), Some(3600));
    assert_eq!(cron_dedupe_seconds(FALLBACK_STATS_JOB_ID), Some(3600));
    assert_eq!(cron_dedupe_seconds(PTU_ROLLUP_JOB_ID), Some(900));
}

#[test]
fn interval_job_window_is_bounded_by_its_own_period() {
    assert_eq!(window_for("interval_job", Some(60), 300), 60);
}

#[test]
fn cron_job_window_is_bounded_by_its_dedupe_horizon() {
    assert_eq!(window_for(PTU_ROLLUP_JOB_ID, None, 1000), 900);
}

#[test]
fn configured_window_wins_when_it_is_the_tightest_bound() {
    assert_eq!(window_for(MONTHLY_SPEND_REPORT_JOB_ID, None, 120), 120);
}
