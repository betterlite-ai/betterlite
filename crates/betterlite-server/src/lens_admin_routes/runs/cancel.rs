use axum::Json;
use axum::extract::{Extension, Path, State};
use betterlite_db::lens_repository::update::DEFAULT_ATTEMPTS;
use betterlite_lens::models::job::{Job, JobStatus, Lens};
use betterlite_lens::sources::storage::Storage;
use chrono::{DateTime, TimeDelta, Utc};

use crate::auth::auth_identity::AuthContext;
use crate::lens_access::lookup::required;
use crate::lens_access::state::{LensDatabase, LensRoutesState, LensStorage};
use crate::lens_admin_routes::error::{LensMutationError, LensRouteError, update_failure};

use super::accessible_repository;

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
