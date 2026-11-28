use std::cmp::max;
use std::collections::BTreeSet;
use std::convert::Infallible;

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, header};
use betterlite_db::lens_repository::update::DEFAULT_ATTEMPTS;
use betterlite_lens::models::execution::Sample;
use betterlite_lens::models::finding::Finding;
use betterlite_lens::models::job::{Job, JobStatus, Lens, Worker};
use betterlite_lens::models::messages::JobResult;
use betterlite_lens::state::snapshot_finding::snapshot_finding;
use chrono::{DateTime, TimeDelta, Utc};

use crate::lens_access::lookup::{get_lens, lens_repository, required};
use crate::lens_access::state::{LensDatabase, LensStorage};
use crate::lens_admin_routes::body::decode;
use crate::lens_inference::jobs::{current_job, replace_job};
use crate::lens_worker_routes::WorkerRoutesState;
use crate::lens_worker_routes::credential::worker_auth;
use crate::lens_worker_routes::error::{LensWorkerError, infallible_update};
use crate::lens_worker_routes::evidence::validate_finding;
use crate::lens_worker_routes::lease::assigned;
use crate::lens_worker_routes::merge::merge_results;
use crate::lens_worker_routes::sample::empty_sample;

pub(crate) async fn result<Db, Store>(
    State(state): State<WorkerRoutesState<Db, Store>>,
    Path((lens_id, job_id)): Path<(String, String)>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<Lens>, LensWorkerError>
where
    Db: LensDatabase,
    Store: LensStorage,
{
    let worker = worker_auth(&state.routes, headers.get(header::AUTHORIZATION)).await?;
    let request: JobResult = decode(&body)?;
    let lens = get_lens(lens_repository(&state.routes)?, &lens_id, &worker.scope).await?;
    if retried(&lens, &job_id, &worker) {
        return Ok(Json(lens));
    }
    let now = state.routes.clock.now();
    let (_, job) = assigned(&state.routes, &lens_id, &job_id, &worker, now).await?;
    let placeholder = empty_sample();
    let selected = job.sample.as_ref().unwrap_or(&placeholder);
    let allowed = selected_ids(selected);
    check_request(&job, &request, &allowed)?;
    for finding in &request.findings {
        validate_finding(&lens, selected, finding, state.routes.storage.as_ref()).await?;
    }
    let apply = |stored: Lens| -> Result<Lens, Infallible> {
        Ok(finished(stored, &request, &job, &worker, now))
    };
    let updated = lens_repository(&state.routes)?
        .update(&lens_id, apply, DEFAULT_ATTEMPTS, false)
        .await
        .map_err(infallible_update)?;
    Ok(Json(required(updated)?))
}

// an upload retried after its run finished gets the stored lens back without touching the job again
fn retried(lens: &Lens, job_id: &str, worker: &Worker) -> bool {
    lens.jobs
        .iter()
        .find(|job| job.id == job_id)
        .is_some_and(|job| {
            matches!(job.status, JobStatus::Completed | JobStatus::Failed)
                && job.worker_id.as_deref() == Some(worker.id.as_str())
        })
}

fn selected_ids(sample: &Sample) -> BTreeSet<&str> {
    sample
        .executions
        .iter()
        .map(|execution| execution.id.as_str())
        .collect()
}

fn check_ids(job: &Job) -> BTreeSet<String> {
    job.settings
        .analysis_checks()
        .into_iter()
        .map(|check| check.id)
        .collect()
}

// inference.result's four 422 checks, run in LiteLLM's order before any storage call
fn check_request(
    job: &Job,
    request: &JobResult,
    allowed: &BTreeSet<&str>,
) -> Result<(), LensWorkerError> {
    let runs: BTreeSet<&str> = request
        .assessments
        .iter()
        .map(|assessment| assessment.execution_id.as_str())
        .collect();
    if runs.len() != request.assessments.len() {
        return Err(LensWorkerError::DuplicateRun);
    }
    if request
        .assessments
        .iter()
        .any(|assessment| !allowed.contains(assessment.execution_id.as_str()))
    {
        return Err(LensWorkerError::RunOutside);
    }
    let checks = check_ids(job);
    let unknown = request.assessments.iter().any(|assessment| {
        assessment
            .issue_checks
            .iter()
            .chain(assessment.pattern_checks.iter())
            .any(|check_id| !checks.contains(check_id.as_str()))
    });
    if unknown {
        return Err(LensWorkerError::CheckUnknown);
    }
    let stray = request.findings.iter().any(|finding| {
        !checks.contains(finding.check_id.as_str())
            || finding
                .evidence
                .iter()
                .any(|evidence| !allowed.contains(evidence.execution_id.as_str()))
    });
    if stray {
        return Err(LensWorkerError::FindingOutside);
    }
    Ok(())
}

// inference.result's finish transform: the job records the run and the lens keeps the merged findings
fn finished(
    lens: Lens,
    request: &JobResult,
    job: &Job,
    worker: &Worker,
    now: DateTime<Utc>,
) -> Lens {
    let Some(active) = current_job(&lens) else {
        return lens;
    };
    if active.id != job.id || active.worker_id.as_deref() != Some(worker.id.as_str()) {
        return lens;
    }
    let merged = merge_results(&lens, &request.findings, job.revision, now).findings;
    let snapshots = draft_snapshots(&lens, request, job.revision, now);
    let failed = !request.error.is_empty();
    let done = finish_job(active, request, &snapshots, now, failed);
    let replaced = replace_job(&lens, &done);
    let findings = merge_old(merged, &lens.findings);
    let last_scan_at = scan_time(&lens, job, failed);
    let next_run_at = next_run(&lens, now);
    Lens {
        jobs: replaced.jobs,
        findings,
        last_scan_at,
        next_run_at,
        ..lens
    }
}

fn draft_snapshots(
    lens: &Lens,
    request: &JobResult,
    revision: i64,
    now: DateTime<Utc>,
) -> Vec<Finding> {
    request
        .findings
        .iter()
        .map(|draft| snapshot_finding(lens, draft, revision, now))
        .collect()
}

fn finish_job(
    active: &Job,
    request: &JobResult,
    snapshots: &[Finding],
    now: DateTime<Utc>,
    failed: bool,
) -> Job {
    let stage = if failed { "Failed" } else { "Complete" };
    Job {
        status: if failed {
            JobStatus::Failed
        } else {
            JobStatus::Completed
        },
        stage: stage.to_owned(),
        finished_at: Some(now),
        coverage: if failed {
            active.coverage.clone()
        } else {
            request.coverage.clone()
        },
        error: request.error.clone(),
        assessments: request.assessments.clone(),
        findings: Some(snapshots.to_vec()),
        ..active.clone()
    }
}

fn merge_old(merged: Vec<Finding>, old: &[Finding]) -> Vec<Finding> {
    let merged_ids: BTreeSet<String> = merged.iter().map(|finding| finding.id.clone()).collect();
    let mut findings = merged;
    findings.extend(
        old.iter()
            .filter(|finding| !merged_ids.contains(&finding.id))
            .cloned(),
    );
    findings
}

fn scan_time(lens: &Lens, job: &Job, failed: bool) -> Option<DateTime<Utc>> {
    if failed {
        return lens.last_scan_at;
    }
    Some(max(lens.last_scan_at.unwrap_or(job.end), job.end))
}

fn next_run(lens: &Lens, now: DateTime<Utc>) -> DateTime<Utc> {
    TimeDelta::try_minutes(i64::from(lens.settings.interval_minutes))
        .and_then(|interval| now.checked_add_signed(interval))
        .unwrap_or(now)
}

mod tests;
