use axum::Json;
use axum::body::Bytes;
use axum::extract::{Extension, Path, State};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use betterlite_db::lens_repository::database::Database;
use betterlite_lens::models::job::{JobStatus, Worker};
use betterlite_lens::sources::storage::Storage;
use betterlite_lens::state::transitions::can_access;
use chrono::{DateTime, Utc};
use garde::Validate;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use crate::auth::auth_identity::AuthContext;
use crate::lens_access::error::LensAccessError;
use crate::lens_access::lookup::lens_repository;
use crate::lens_access::scope::{caller_key, user_scope};
use crate::lens_access::state::LensRoutesState;
use crate::lens_admin_routes::body::decode;
use crate::lens_admin_routes::error::LensRouteError;
use crate::lens_billing::key_validation::validate_key;

const TOKEN_BYTES: usize = 40;
const WORKER_NAME: &str = "Lens worker";

#[derive(Debug, Deserialize, Validate)]
#[serde(deny_unknown_fields)]
#[garde(allow_unvalidated)]
pub(crate) struct WorkerBilling {
    #[garde(pattern(r"^[a-f0-9]{64}$"))]
    pub(crate) analysis_key_id: String,
}

#[derive(Debug, Deserialize, Validate)]
#[serde(deny_unknown_fields)]
#[garde(allow_unvalidated)]
pub(crate) struct WorkerName {
    #[garde(pattern(r"^[a-f0-9]{64}$"))]
    pub(crate) analysis_key_id: String,
    #[serde(default = "default_name")]
    #[garde(length(chars, min = 1, max = 100))]
    pub(crate) name: String,
}

// the token is returned once on registration and stored only as its SHA-256 digest
#[derive(Serialize)]
pub(crate) struct RegisteredWorker {
    pub(crate) worker: Worker,
    pub(crate) token: SecretString,
}

fn default_name() -> String {
    WORKER_NAME.to_owned()
}

pub(crate) async fn register_worker<Db, Store>(
    State(state): State<LensRoutesState<Db, Store>>,
    caller: Option<Extension<AuthContext>>,
    body: Bytes,
) -> Result<Json<RegisteredWorker>, LensRouteError<<Store as Storage>::Error>>
where
    Db: Database + 'static,
    Store: Storage + Clone + Send + Sync + 'static,
{
    let Some(Extension(context)) = caller else {
        return Err(LensRouteError::Unauthenticated);
    };
    let request: WorkerName = decode(&body)?;
    let key = caller_key(&context.principal)?;
    let scope = user_scope(&key, true)?;
    validate_key(state.keys.as_ref(), Some(&request.analysis_key_id)).await?;
    let encoded = URL_SAFE_NO_PAD.encode(state.ids.random_bytes(TOKEN_BYTES));
    let token = format!("lens-{encoded}");
    let worker = Worker {
        analysis_key_id: Some(request.analysis_key_id.clone()),
        id: state.ids.new_id(),
        name: request.name,
        scope,
        last_seen: DateTime::<Utc>::UNIX_EPOCH,
        revoked: false,
    };
    let digest = hex::encode(Sha256::digest(token.as_bytes()));
    let repository = lens_repository(&state)?;
    repository.save_worker(&worker, Some(&digest)).await?;
    Ok(Json(RegisteredWorker {
        worker,
        token: SecretString::from(token),
    }))
}

pub(crate) async fn set_worker_billing<Db, Store>(
    State(state): State<LensRoutesState<Db, Store>>,
    Path(worker_id): Path<String>,
    caller: Option<Extension<AuthContext>>,
    body: Bytes,
) -> Result<Json<Worker>, LensRouteError<<Store as Storage>::Error>>
where
    Db: Database + 'static,
    Store: Storage + Clone + Send + Sync + 'static,
{
    let Some(Extension(context)) = caller else {
        return Err(LensRouteError::Unauthenticated);
    };
    let request: WorkerBilling = decode(&body)?;
    let scope = user_scope(&caller_key(&context.principal)?, true)?;
    let repository = lens_repository(&state)?;
    let workers = repository.workers().await?;
    let Some(worker) = workers
        .into_iter()
        .find(|worker| worker.id == worker_id && can_access(&scope, &worker.scope))
    else {
        return Err(LensRouteError::Access(LensAccessError::WorkerNotFound));
    };
    if worker.revoked {
        return Err(LensRouteError::Access(LensAccessError::WorkerRevoked));
    }
    validate_key(state.keys.as_ref(), Some(&request.analysis_key_id)).await?;
    let updated = repository
        .set_worker_billing(&worker.id, &request.analysis_key_id)
        .await?;
    let Some(updated) = updated else {
        return Err(LensRouteError::Access(LensAccessError::WorkerAccessRevoked));
    };
    Ok(Json(updated))
}

pub(crate) async fn revoke_worker<Db, Store>(
    State(state): State<LensRoutesState<Db, Store>>,
    Path(worker_id): Path<String>,
    caller: Option<Extension<AuthContext>>,
) -> Result<Json<bool>, LensRouteError<<Store as Storage>::Error>>
where
    Db: Database + 'static,
    Store: Storage + Clone + Send + Sync + 'static,
{
    let Some(Extension(context)) = caller else {
        return Err(LensRouteError::Unauthenticated);
    };
    let scope = user_scope(&caller_key(&context.principal)?, true)?;
    let repository = lens_repository(&state)?;
    let workers = repository.workers().await?;
    let Some(worker) = workers
        .into_iter()
        .find(|worker| worker.id == worker_id && can_access(&scope, &worker.scope))
    else {
        return Err(LensRouteError::Access(LensAccessError::WorkerNotFound));
    };
    let lenses = repository.lenses().await?;
    let busy = lenses.iter().flat_map(|lens| lens.jobs.iter()).any(|job| {
        job.status == JobStatus::Running && job.worker_id.as_deref() == Some(worker.id.as_str())
    });
    if busy {
        return Err(LensRouteError::Access(LensAccessError::WorkerRunning));
    }
    repository.revoke_worker(&worker.id).await?;
    Ok(Json(true))
}

mod tests;
