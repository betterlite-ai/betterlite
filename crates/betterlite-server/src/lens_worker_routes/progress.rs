use std::convert::Infallible;

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, header};
use betterlite_db::lens_repository::update::DEFAULT_ATTEMPTS;
use betterlite_lens::models::job::{Job, Lens, Worker};
use betterlite_lens::models::messages::Progress;
use chrono::{DateTime, TimeDelta, Utc};

use crate::lens_access::lookup::{lens_repository, required};
use crate::lens_access::state::{LensDatabase, LensRoutesState, LensStorage};
use crate::lens_admin_routes::body::decode;
use crate::lens_inference::jobs::{current_job, replace_job};
use crate::lens_worker_routes::WorkerRoutesState;
use crate::lens_worker_routes::credential::worker_auth;
use crate::lens_worker_routes::error::{LensWorkerError, infallible_update};
use crate::lens_worker_routes::lease::assigned;

pub(crate) async fn progress<Db, Store>(
    State(state): State<WorkerRoutesState<Db, Store>>,
    Path((lens_id, job_id)): Path<(String, String)>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<bool>, LensWorkerError>
where
    Db: LensDatabase,
    Store: LensStorage,
{
    let worker = worker_auth(&state.routes, headers.get(header::AUTHORIZATION)).await?;
    let request: Progress = decode(&body)?;
    apply(&state.routes, &lens_id, &job_id, &worker, &request).await?;
    Ok(Json(true))
}

pub(crate) async fn heartbeat<Db, Store>(
    State(state): State<WorkerRoutesState<Db, Store>>,
    Path((lens_id, job_id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Json<bool>, LensWorkerError>
where
    Db: LensDatabase,
    Store: LensStorage,
{
    let worker = worker_auth(&state.routes, headers.get(header::AUTHORIZATION)).await?;
    let now = state.routes.clock.now();
    let (_, job) = assigned(&state.routes, &lens_id, &job_id, &worker, now).await?;
    let body = Progress {
        stage: job.stage.clone(),
        coverage: job.coverage.clone(),
    };
    apply(&state.routes, &lens_id, &job_id, &worker, &body).await?;
    Ok(Json(true))
}

// progress's handler body, which heartbeat reaches again with the job's own stage and coverage
async fn apply<Db, Store>(
    state: &LensRoutesState<Db, Store>,
    lens_id: &str,
    job_id: &str,
    worker: &Worker,
    body: &Progress,
) -> Result<(), LensWorkerError>
where
    Db: LensDatabase,
{
    let now = state.clock.now();
    assigned(state, lens_id, job_id, worker, now).await?;
    let renew = |lens: Lens| -> Result<Lens, Infallible> {
        Ok(renewed(lens, job_id, &worker.id, body, now))
    };
    let repository = lens_repository(state)?;
    let updated = repository
        .update(lens_id, renew, DEFAULT_ATTEMPTS, false)
        .await
        .map_err(infallible_update)?;
    required(updated)?;
    repository.heartbeat(&worker.id, &isoformat(now)).await?;
    Ok(())
}

// a job that left this worker between the lease check and the write keeps the lens as it is
fn renewed(lens: Lens, job_id: &str, worker_id: &str, body: &Progress, now: DateTime<Utc>) -> Lens {
    let Some(job) = current_job(&lens) else {
        return lens;
    };
    if job.id != job_id || job.worker_id.as_deref() != Some(worker_id) {
        return lens;
    }
    let lease_until = now.checked_add_signed(TimeDelta::minutes(5)).unwrap_or(now);
    let updated = Job {
        stage: body.stage.clone(),
        coverage: body.coverage.clone(),
        lease_until: Some(lease_until),
        ..job.clone()
    };
    replace_job(&lens, &updated)
}

// Python's datetime.isoformat for an aware UTC value: microseconds only when they are not zero
pub(crate) fn isoformat(now: DateTime<Utc>) -> String {
    let seconds = now.format("%Y-%m-%dT%H:%M:%S");
    let micros = now.timestamp_subsec_micros();
    if micros == 0 {
        return format!("{seconds}+00:00");
    }
    format!("{seconds}.{micros:06}+00:00")
}

mod tests;
