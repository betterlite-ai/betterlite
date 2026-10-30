#![cfg(test)]

use chrono::{DateTime, Utc};

use super::is_staggerable;
use crate::scheduled_job_stagger::{JobSnapshot, MONTHLY_SPEND_REPORT_JOB_ID, Trigger};

struct NeverFires;

impl Trigger for NeverFires {
    fn next_fire_time(
        &self,
        _previous_fire_time: Option<DateTime<Utc>>,
        _now: DateTime<Utc>,
    ) -> Option<DateTime<Utc>> {
        None
    }
}

fn job(id: &str, next_run_time: Option<DateTime<Utc>>, period_seconds: Option<u32>) -> JobSnapshot {
    JobSnapshot {
        id: id.to_owned(),
        next_run_time,
        period_seconds,
        trigger: Box::new(NeverFires),
    }
}

#[test]
fn job_with_an_explicit_anchor_is_never_staggerable() {
    assert!(!is_staggerable(&job(
        "interval_job",
        Some(Utc::now()),
        Some(60)
    )));
}

#[test]
fn interval_job_is_always_staggerable() {
    assert!(is_staggerable(&job("interval_job", None, Some(60))));
}

#[test]
fn default_cron_job_is_staggerable() {
    assert!(is_staggerable(&job(
        MONTHLY_SPEND_REPORT_JOB_ID,
        None,
        None
    )));
}

#[test]
fn unlisted_cron_job_is_not_staggerable() {
    assert!(!is_staggerable(&job("some_other_cron_job", None, None)));
}
