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
    Ok(Json(runs_page(&state, caller, lens_id, raw).await?))
}

async fn runs_page<Db, Store>(
    state: &LensRoutesState<Db, Store>,
    caller: Option<Extension<AuthContext>>,
    lens_id: String,
    raw: RawQuery,
) -> Result<Vec<Job>, LensRouteError<<Store as Storage>::Error>>
where
    Db: LensDatabase,
    Store: LensStorage,
{
    let query = content_query(raw)?;
    let repository = state.accessible_repository(caller, &lens_id, false).await?;
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
    Ok(stripped)
}

impl<Db, Store> LensRoutesState<Db, Store>
where
    Db: LensDatabase,
    Store: LensStorage,
{
    pub(crate) async fn run_lens(
        State(state): State<Self>,
        Path(lens_id): Path<String>,
        caller: Option<Extension<AuthContext>>,
        body: Bytes,
    ) -> Result<Json<Lens>, LensRouteError<<Store as Storage>::Error>> {
        Ok(Json(state.queue_run(caller, lens_id, body).await?))
    }

    pub(crate) async fn read_run(
        State(state): State<Self>,
        Path((lens_id, job_id)): Path<(String, String)>,
        caller: Option<Extension<AuthContext>>,
    ) -> Result<Json<Job>, LensRouteError<<Store as Storage>::Error>> {
        Ok(Json(state.run_of(caller, lens_id, job_id).await?))
    }

    pub(crate) async fn cancel_lens(
        State(state): State<Self>,
        Path(lens_id): Path<String>,
        caller: Option<Extension<AuthContext>>,
    ) -> Result<Json<Lens>, LensRouteError<<Store as Storage>::Error>> {
        Ok(Json(state.cancel_now(caller, lens_id).await?))
    }

    async fn queue_run(
        &self,
        caller: Option<Extension<AuthContext>>,
        lens_id: String,
        body: Bytes,
    ) -> Result<Lens, LensRouteError<<Store as Storage>::Error>> {
        let request: RunRequest = decode(&body)?;
        let (key, scope) = route_key_scope::<Store>(caller, true)?;
        let repository = lens_repository(self)?;
        get_lens(repository, &lens_id, &scope).await?;
        if let Some(settings) = request.settings.as_ref() {
            validate_model(settings, &key, self)?;
        }
        let now = self.clock.now();
        let job_id = self.ids.new_id();
        let updated = repository
            .update(
                &lens_id,
                |lens| {
                    queue_job(&lens, now, job_id.clone(), &request).map_err(LensMutationError::from)
                },
                DEFAULT_ATTEMPTS,
                false,
            )
            .await
            .map_err(update_failure)?;
        Ok(required(updated)?)
    }

    async fn run_of(
        &self,
        caller: Option<Extension<AuthContext>>,
        lens_id: String,
        job_id: String,
    ) -> Result<Job, LensRouteError<<Store as Storage>::Error>> {
        let repository = self.accessible_repository(caller, &lens_id, false).await?;
        let Some(job) = repository.job(&lens_id, &job_id).await? else {
            return Err(LensRouteError::Access(
                LensAccessError::InvestigationNotFound,
            ));
        };
        Ok(job)
    }

    async fn cancel_now(
        &self,
        caller: Option<Extension<AuthContext>>,
        lens_id: String,
    ) -> Result<Lens, LensRouteError<<Store as Storage>::Error>> {
        let repository = self.accessible_repository(caller, &lens_id, true).await?;
        let now = self.clock.now();
        let updated = repository
            .update(
                &lens_id,
                |lens| cancelled(lens, now),
                DEFAULT_ATTEMPTS,
                false,
            )
            .await
            .map_err(update_failure)?;
        Ok(required(updated)?)
    }

    // the repository, once the caller's scope covers this lens: every run route starts here
    async fn accessible_repository(
        &self,
        caller: Option<Extension<AuthContext>>,
        lens_id: &str,
        write: bool,
    ) -> Result<&LensRepository<Db>, LensRouteError<<Store as Storage>::Error>> {
        let (_, scope) = route_key_scope::<Store>(caller, write)?;
        let repository = lens_repository(self)?;
        get_lens(repository, lens_id, &scope).await?;
        Ok(repository)
    }
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
