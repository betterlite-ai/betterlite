use chrono::{DateTime, TimeDelta, Utc};
use rust_decimal::Decimal;
use thiserror::Error;

use crate::models::execution::Coverage;
use crate::models::job::{Job, JobStatus, Lens, Worker};
use crate::models::messages::RunRequest;
use crate::models::settings::Scope;

mod tests;

const MAX_ATTEMPTS: i64 = 3;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum TransitionError {
    #[error("the job's {field} time falls outside the supported date range")]
    Time { field: &'static str },
    #[error("the job's attempt count {attempts} cannot grow")]
    Attempts { attempts: i64 },
}

#[must_use]
pub fn can_access(viewer: &Scope, target: &Scope) -> bool {
    if viewer.all_teams {
        true
    } else if target.all_teams || viewer.team_id != target.team_id {
        false
    } else {
        !viewer.team_id.is_empty() || viewer.api_key_hash == target.api_key_hash
    }
}

pub fn queue_job(
    lens: &Lens,
    now: DateTime<Utc>,
    job_id: String,
    request: &RunRequest,
) -> Result<Lens, TransitionError> {
    if lens
        .jobs
        .iter()
        .any(|job| matches!(job.status, JobStatus::Queued | JobStatus::Running))
    {
        return Ok(lens.clone());
    }
    let settings = request.settings.as_ref().unwrap_or(&lens.settings);
    let hours = request.lookback_hours.unwrap_or(settings.lookback_hours);
    let start = TimeDelta::try_hours(i64::from(hours))
        .and_then(|lookback| now.checked_sub_signed(lookback))
        .ok_or(TransitionError::Time { field: "start" })?;
    // executions still arriving in the last two minutes wait for the next run
    let end = now
        .checked_sub_signed(TimeDelta::minutes(2))
        .ok_or(TransitionError::Time { field: "end" })?;
    let job = Job {
        id: job_id,
        status: JobStatus::Queued,
        stage: "Queued".to_owned(),
        created_at: now,
        start,
        end,
        settings: settings.clone(),
        revision: lens.revision,
        worker_id: None,
        lease_until: None,
        attempts: 0,
        finished_at: None,
        coverage: Coverage::default(),
        error: String::new(),
        sample: None,
        cost: Decimal::ZERO,
        findings: None,
        assessments: Vec::new(),
    };
    Ok(Lens {
        jobs: vec![job],
        ..lens.clone()
    })
}

pub fn claim_job(
    lens: &Lens,
    worker: &Worker,
    now: DateTime<Utc>,
) -> Result<Lens, TransitionError> {
    let current = lens
        .jobs
        .iter()
        .find(|job| matches!(job.status, JobStatus::Queued | JobStatus::Running));
    let Some(job) = current.filter(|_| can_access(&worker.scope, &lens.scope)) else {
        return Ok(lens.clone());
    };
    if job.status == JobStatus::Running && job.lease_until.is_some_and(|lease| lease > now) {
        return Ok(lens.clone());
    }
    let (claimed, next_run_at) = if job.attempts >= MAX_ATTEMPTS {
        give_up(lens, job, now)?
    } else {
        (lease(job, worker, now)?, lens.next_run_at)
    };
    let jobs = lens
        .jobs
        .iter()
        .map(|old| if old.id == claimed.id { &claimed } else { old });
    Ok(Lens {
        jobs: jobs.cloned().collect(),
        next_run_at,
        ..lens.clone()
    })
}

fn give_up(
    lens: &Lens,
    job: &Job,
    now: DateTime<Utc>,
) -> Result<(Job, DateTime<Utc>), TransitionError> {
    let next_run_at = TimeDelta::try_minutes(i64::from(lens.settings.interval_minutes))
        .and_then(|interval| now.checked_add_signed(interval))
        .ok_or(TransitionError::Time {
            field: "next_run_at",
        })?;
    let failed = Job {
        status: JobStatus::Failed,
        stage: "Failed".to_owned(),
        error: "Worker disconnected repeatedly".to_owned(),
        finished_at: Some(now),
        ..job.clone()
    };
    Ok((failed, next_run_at))
}

fn lease(job: &Job, worker: &Worker, now: DateTime<Utc>) -> Result<Job, TransitionError> {
    let lease_until =
        now.checked_add_signed(TimeDelta::minutes(5))
            .ok_or(TransitionError::Time {
                field: "lease_until",
            })?;
    let attempts = job
        .attempts
        .checked_add(1)
        .ok_or(TransitionError::Attempts {
            attempts: job.attempts,
        })?;
    Ok(Job {
        status: JobStatus::Running,
        stage: "Collecting executions".to_owned(),
        worker_id: Some(worker.id.clone()),
        lease_until: Some(lease_until),
        attempts,
        ..job.clone()
    })
}

#[must_use]
pub fn renew_budget(lens: &Lens, now: DateTime<Utc>) -> Lens {
    let month = now.format("%Y-%m").to_string();
    if lens.budget_month == month {
        return lens.clone();
    }
    Lens {
        budget_month: month,
        spent: Decimal::ZERO,
        ..lens.clone()
    }
}
