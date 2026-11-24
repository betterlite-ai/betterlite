use axum::Json;
use axum::body::Bytes;
use axum::extract::{Extension, Path, RawQuery, State};
use betterlite_db::lens_repository::repository::LensRepository;
use betterlite_db::lens_repository::update::DEFAULT_ATTEMPTS;
use betterlite_lens::models::job::{Job, JobStatus, Lens};
use betterlite_lens::models::messages::RunRequest;
use betterlite_lens::sources::storage::Storage;
use betterlite_lens::state::transitions::queue_job;
use chrono::{DateTime, TimeDelta, Utc};

use crate::auth::auth_identity::AuthContext;
use crate::lens_access::error::LensAccessError;
use crate::lens_access::lookup::{get_lens, lens_repository, required};
use crate::lens_access::model::validate_model;
use crate::lens_access::state::{LensDatabase, LensRoutesState, LensStorage};
use crate::lens_admin_routes::body::{content_query, decode};
use crate::lens_admin_routes::error::{
    LensMutationError, LensRouteError, route_key_scope, update_failure,
};

pub(crate) async fn list_runs<Db, Store>(
    State(state): State<LensRoutesState<Db, Store>>,
    Path(lens_id): Path<String>,
    caller: Option<Extension<AuthContext>>,
    raw: RawQuery,
) -> Result<Json<Vec<Job>>, LensRouteError<<Store as Storage>::Error>>
where
    Db: LensDatabase,
    Store: LensStorage,
{
    let query = content_query(raw)?;
    let repository = accessible_repository(&state, caller, &lens_id, false).await?;
    let jobs = repository.jobs(&lens_id, query.offset).await?;
    let stripped = jobs
        .into_iter()
        .map(|job| Job {
            sample: None,
            findings: None,
            assessments: Vec::new(),
            ..job
        })
        .collect();
    Ok(Json(stripped))
}

pub(crate) async fn run_lens<Db, Store>(
    State(state): State<LensRoutesState<Db, Store>>,
    Path(lens_id): Path<String>,
    caller: Option<Extension<AuthContext>>,
    body: Bytes,
) -> Result<Json<Lens>, LensRouteError<<Store as Storage>::Error>>
where
    Db: LensDatabase,
    Store: LensStorage,
{
    let request: RunRequest = decode(&body)?;
    let (key, scope) = route_key_scope::<Store>(caller, true)?;
    let repository = lens_repository(&state)?;
    get_lens(repository, &lens_id, &scope).await?;
    if let Some(settings) = request.settings.as_ref() {
        validate_model(settings, &key, &state)?;
    }
    let now = state.clock.now();
    let job_id = state.ids.new_id();
    let updated = repository
        .update(
            &lens_id,
            |lens| queue_job(&lens, now, job_id.clone(), &request).map_err(LensMutationError::from),
            DEFAULT_ATTEMPTS,
            false,
        )
        .await
        .map_err(update_failure)?;
    Ok(Json(required(updated)?))
}

pub(crate) async fn read_run<Db, Store>(
    State(state): State<LensRoutesState<Db, Store>>,
    Path((lens_id, job_id)): Path<(String, String)>,
    caller: Option<Extension<AuthContext>>,
) -> Result<Json<Job>, LensRouteError<<Store as Storage>::Error>>
where
    Db: LensDatabase,
    Store: LensStorage,
{
    let repository = accessible_repository(&state, caller, &lens_id, false).await?;
    repository
        .job(&lens_id, &job_id)
        .await?
        .map(Json)
        .ok_or(LensRouteError::Access(
            LensAccessError::InvestigationNotFound,
        ))
}

// the repository, once the caller's scope covers this lens: every run route starts here
async fn accessible_repository<'a, Db, Store>(
    state: &'a LensRoutesState<Db, Store>,
    caller: Option<Extension<AuthContext>>,
    lens_id: &str,
    write: bool,
) -> Result<&'a LensRepository<Db>, LensRouteError<<Store as Storage>::Error>>
where
    Db: LensDatabase,
    Store: LensStorage,
{
    let (_, scope) = route_key_scope::<Store>(caller, write)?;
    let repository = lens_repository(state)?;
    get_lens(repository, lens_id, &scope).await?;
    Ok(repository)
}

pub(crate) async fn cancel_lens<Db, Store>(
    State(state): State<LensRoutesState<Db, Store>>,
    Path(lens_id): Path<String>,
    caller: Option<Extension<AuthContext>>,
) -> Result<Json<Lens>, LensRouteError<<Store as Storage>::Error>>
where
    Db: LensDatabase,
    Store: LensStorage,
{
    let now = state.clock.now();
    let repository = accessible_repository(&state, caller, &lens_id, true).await?;
    let updated = repository
        .update(
            &lens_id,
            |lens| cancelled(lens, now),
            DEFAULT_ATTEMPTS,
            false,
        )
        .await
        .map_err(update_failure)?;
    Ok(Json(required(updated)?))
}

fn cancelled(lens: Lens, now: DateTime<Utc>) -> Result<Lens, LensMutationError> {
    let current = lens
        .jobs
        .iter()
        .find(|job| matches!(job.status, JobStatus::Queued | JobStatus::Running));
    let Some(job) = current.cloned() else {
        return Ok(lens);
    };
    let cancelled_job = Job {
        status: JobStatus::Cancelled,
        stage: "Cancelled".to_owned(),
        finished_at: Some(now),
        ..job
    };
    let next_run_at = TimeDelta::try_minutes(i64::from(lens.settings.interval_minutes))
        .and_then(|interval| now.checked_add_signed(interval))
        .ok_or(LensMutationError::Deadline)?;
    let jobs = lens
        .jobs
        .iter()
        .map(|old| {
            if old.id == cancelled_job.id {
                cancelled_job.clone()
            } else {
                old.clone()
            }
        })
        .collect();
    Ok(Lens {
        jobs,
        next_run_at,
        ..lens
    })
}

mod tests;
