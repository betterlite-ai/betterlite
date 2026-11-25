use axum::Json;
use axum::body::Bytes;
use axum::extract::{Extension, Path, State};
use betterlite_lens::models::job::Worker;
use betterlite_lens::sources::storage::Storage;

use crate::auth::auth_identity::AuthContext;
use crate::lens_access::error::LensAccessError;
use crate::lens_access::state::{LensDatabase, LensRoutesState, LensStorage};
use crate::lens_admin_routes::body::decode;
use crate::lens_admin_routes::error::LensRouteError;
use crate::lens_billing::key_validation::validate_key;

use super::{WorkerBilling, scoped_worker_for};

pub(crate) async fn set_worker_billing<Db, Store>(
    State(state): State<LensRoutesState<Db, Store>>,
    Path(worker_id): Path<String>,
    caller: Option<Extension<AuthContext>>,
    body: Bytes,
) -> Result<Json<Worker>, LensRouteError<<Store as Storage>::Error>>
where
    Db: LensDatabase,
    Store: LensStorage,
{
    let request: WorkerBilling = decode(&body)?;
    let (repository, worker) = scoped_worker_for(&state, caller, &worker_id).await?;
    if worker.revoked {
        return Err(LensRouteError::Access(LensAccessError::WorkerRevoked));
    }
    validate_key(state.keys.as_ref(), Some(&request.analysis_key_id)).await?;
    let updated = repository
        .set_worker_billing(&worker.id, &request.analysis_key_id)
        .await?;
    let Some(patched) = updated else {
        return Err(LensRouteError::Access(LensAccessError::WorkerAccessRevoked));
    };
    Ok(Json(patched))
}
