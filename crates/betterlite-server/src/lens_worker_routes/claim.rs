use std::convert::Infallible;
use std::num::ParseIntError;

use axum::Json;
use axum::extract::{RawQuery, State};
use axum::http::{HeaderMap, header};
use betterlite_db::lens_repository::update::DEFAULT_ATTEMPTS;
use betterlite_db::search_tools::convert::isoformat;
use betterlite_lens::models::job::{JobStatus, Lens, Worker};
use betterlite_lens::models::messages::{Claim, RunRequest};
use betterlite_lens::sources::storage::Storage;
use betterlite_lens::state::transitions::{can_access, claim_job, queue_job};
use chrono::{DateTime, Utc};

use crate::lens_access::lookup::lens_repository;
use crate::lens_access::state::{LensDatabase, LensStorage};
use crate::lens_inference::jobs::current_job;
use crate::lens_worker_routes::WorkerRoutesState;
use crate::lens_worker_routes::credential::worker_auth;
use crate::lens_worker_routes::error::{LensWorkerError, infallible_update, query_refusal};

const INTEGER_PARSE: &str = "Input should be a valid integer, unable to parse string as an integer";

pub(crate) async fn claim<Db, Store>(
    State(state): State<WorkerRoutesState<Db, Store>>,
    headers: HeaderMap,
    raw: RawQuery,
) -> Result<Json<Option<Claim>>, LensWorkerError<<Store as Storage>::Error>>
where
    Db: LensDatabase,
    Store: LensStorage,
{
    let worker = worker_auth(&state.routes, headers.get(header::AUTHORIZATION)).await?;
    let protocol = protocol_version(raw.0.as_deref().unwrap_or_default().as_bytes())
        .map_err(|_error| query_refusal("int_parsing", "protocol_version", INTEGER_PARSE))?;
    if protocol != 2 {
        return Err(LensWorkerError::UpgradeWorker);
    }
    if worker.analysis_key_id.is_none() {
        return Err(LensWorkerError::NoAnalysisKey);
    }
    let now = state.routes.clock.now();
    let repository = lens_repository(&state.routes)?;
    repository.heartbeat(&worker.id, &isoformat(now)).await?;
    for candidate in repository.lenses().await? {
        if !can_access(&worker.scope, &candidate.scope) {
            continue;
        }
        if let Some(claim) = claim_candidate(&state, &candidate, &worker, now).await? {
            return Ok(Json(Some(claim)));
        }
    }
    Ok(Json(None))
}

// protocol_version is an int query parameter defaulting to 1, decoded after the worker's own auth
pub fn protocol_version(raw: &[u8]) -> Result<i64, ParseIntError> {
    let mut version = 1_i64;
    for (name, value) in form_urlencoded::parse(raw).into_owned() {
        if name != "protocol_version" {
            continue;
        }
        version = value.parse::<i64>()?;
    }
    Ok(version)
}

// claim_candidate: queue a due job behind changed_only, so a lens with nothing to give back is no work
async fn claim_candidate<Db, Store>(
    state: &WorkerRoutesState<Db, Store>,
    candidate: &Lens,
    worker: &Worker,
    now: DateTime<Utc>,
) -> Result<Option<Claim>, LensWorkerError<<Store as Storage>::Error>>
where
    Db: LensDatabase,
    Store: LensStorage,
{
    let job_id = state.routes.ids.new_id();
    let schedule =
        |lens: Lens| -> Result<Lens, Infallible> { Ok(scheduled(lens, worker, now, &job_id)) };
    let repository = lens_repository(&state.routes)?;
    let updated = repository
        .update(&candidate.id, schedule, DEFAULT_ATTEMPTS, true)
        .await
        .map_err(infallible_update)?;
    let Some(changed) = updated else {
        return Ok(None);
    };
    let Some(job) = current_job(&changed).cloned() else {
        return Ok(None);
    };
    let claimed = job.worker_id.as_deref() == Some(worker.id.as_str())
        && job.status == JobStatus::Running
        && current_job(candidate) != Some(&job);
    if !claimed {
        return Ok(None);
    }
    Ok(Some(Claim {
        lens_id: changed.id,
        job,
        findings: changed.findings,
    }))
}

// queue_job only when the lens is enabled and due, then claim_job over the result
fn scheduled(lens: Lens, worker: &Worker, now: DateTime<Utc>, job_id: &str) -> Lens {
    let queued = if lens.settings.enabled && lens.next_run_at <= now {
        match queue_job(&lens, now, job_id.to_owned(), &RunRequest::default()) {
            Ok(queued) => queued,
            Err(_error) => lens,
        }
    } else {
        lens
    };
    match claim_job(&queued, worker, now) {
        Ok(claimed) => claimed,
        Err(_error) => queued,
    }
}

mod tests;
