use std::convert::Infallible;

use axum::Json;
use axum::body::Bytes;
use axum::extract::{MatchedPath, Path, State};
use axum::http::{HeaderMap, header};
use betterlite_db::lens_repository::update::DEFAULT_ATTEMPTS;
use betterlite_db::search_tools::convert::isoformat;
use betterlite_lens::models::job::{Job, Lens, Worker};
use betterlite_lens::models::messages::Progress;
use betterlite_lens::sources::storage::Storage;
use chrono::{DateTime, TimeDelta, Utc};

use crate::lens_access::lookup::{lens_repository, required};
use crate::lens_access::state::{LensDatabase, LensRoutesState, LensStorage};
use crate::lens_admin_routes::body::decode;
use crate::lens_inference::jobs::{current_job, replace_job};
use crate::lens_worker_routes::WorkerRoutesState;
use crate::lens_worker_routes::credential::worker_auth;
use crate::lens_worker_routes::error::{LensWorkerError, infallible_update};
use crate::lens_worker_routes::lease::assigned;

// LiteLLM's heartbeat calls progress with the job's stage, so one handler serves both routes
pub(crate) async fn progress<Db, Store>(
    State(state): State<WorkerRoutesState<Db, Store>>,
    Path((lens_id, job_id)): Path<(String, String)>,
    matched: MatchedPath,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<bool>, LensWorkerError<<Store as Storage>::Error>>
where
    Db: LensDatabase,
    Store: LensStorage,
{
    let worker = worker_auth(&state.routes, headers.get(header::AUTHORIZATION)).await?;
    let request: Progress = if matched.as_str().ends_with("/heartbeat") {
        carried(&state.routes, &lens_id, &job_id, &worker).await?
    } else {
        decode(&body)?
    };
    apply(&state.routes, &lens_id, &job_id, &worker, &request).await?;
    Ok(Json(true))
}

// heartbeat's own assigned check, before progress's check runs again inside apply
async fn carried<Db, Store>(
    state: &LensRoutesState<Db, Store>,
    lens_id: &str,
    job_id: &str,
    worker: &Worker,
) -> Result<Progress, LensWorkerError<<Store as Storage>::Error>>
where
    Db: LensDatabase,
    Store: LensStorage,
{
    let now = state.clock.now();
    let (_, job) = assigned(state, lens_id, job_id, worker, now).await?;
    Ok(Progress {
        stage: job.stage,
        coverage: job.coverage,
    })
}

// progress's handler body, which heartbeat reaches again with the job's own stage and coverage
async fn apply<Db, Store>(
    state: &LensRoutesState<Db, Store>,
    lens_id: &str,
    job_id: &str,
    worker: &Worker,
    body: &Progress,
) -> Result<(), LensWorkerError<<Store as Storage>::Error>>
where
    Db: LensDatabase,
    Store: LensStorage,
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

mod tests;
