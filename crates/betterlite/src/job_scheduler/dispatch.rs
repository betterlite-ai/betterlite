use std::sync::Arc;

use chrono::{DateTime, TimeDelta, Utc};

use crate::scheduled_job_stagger::{JobSubmission, Trigger};

use super::{Clock, JobFn};

const MAX_DUE_RUNS: usize = 10_000;

pub(crate) struct SubmissionEvent {
    pub(crate) job_id: String,
    pub(crate) run_times: Vec<DateTime<Utc>>,
}

impl JobSubmission for SubmissionEvent {
    fn job_id(&self) -> &str {
        &self.job_id
    }

    fn scheduled_run_times(&self) -> &[DateTime<Utc>] {
        &self.run_times
    }
}

pub(crate) fn due_run_times(
    trigger: &dyn Trigger,
    next_fire: DateTime<Utc>,
    now: DateTime<Utc>,
) -> Vec<DateTime<Utc>> {
    let mut times = Vec::new();
    let mut current = next_fire;
    loop {
        if current > now {
            break;
        }
        times.push(current);
        if times.len() >= MAX_DUE_RUNS {
            tracing::warn!(
                "scheduled job has more than {MAX_DUE_RUNS} due runs; keeping the first {MAX_DUE_RUNS}"
            );
            break;
        }
        match trigger.next_fire_time(Some(current), now) {
            Some(next) if next > current => current = next,
            Some(next) => {
                tracing::warn!(
                    next = %next,
                    "scheduled job trigger did not advance; stopping its due runs"
                );
                break;
            }
            None => break,
        }
    }
    times
}

pub(crate) async fn execute_run(
    run: JobFn,
    job_id: String,
    run_times: Vec<DateTime<Utc>>,
    grace_seconds: u64,
    clock: Arc<dyn Clock>,
) {
    let grace = i64::try_from(grace_seconds)
        .ok()
        .and_then(TimeDelta::try_seconds);
    for run_time in run_times {
        let delay = clock.now().signed_duration_since(run_time);
        if grace.is_some_and(|limit| delay > limit) {
            tracing::warn!("Run time of job {job_id} was missed by {delay}");
            continue;
        }
        tracing::info!("Running job {job_id} (scheduled at {run_time})");
        (run)().await;
    }
}
