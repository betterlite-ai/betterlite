use axum::Json;
use axum::extract::{Extension, Path, State};
use betterlite_lens::models::job::Job;
use betterlite_lens::sources::storage::Storage;

use crate::auth::auth_identity::AuthContext;
use crate::lens_access::error::LensAccessError;
use crate::lens_access::state::{LensDatabase, LensRoutesState, LensStorage};
use crate::lens_admin_routes::error::LensRouteError;

use super::accessible_repository;

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
    let Some(found) = repository.job(&lens_id, &job_id).await? else {
        return Err(LensRouteError::Access(
            LensAccessError::InvestigationNotFound,
        ));
    };
    Ok(Json(found))
}
