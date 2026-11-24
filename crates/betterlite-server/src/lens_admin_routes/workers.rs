use axum::Json;
use axum::body::Bytes;
use axum::extract::{Extension, Path, State};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use betterlite_db::lens_repository::repository::LensRepository;
use betterlite_lens::models::job::{JobStatus, Worker};
use betterlite_lens::sources::storage::Storage;
use betterlite_lens::state::transitions::can_access;
use chrono::{DateTime, Utc};
use garde::Validate;
use secrecy::{ExposeSecret as _, SecretString};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use crate::auth::auth_identity::AuthContext;
use crate::lens_access::error::LensAccessError;
use crate::lens_access::lookup::lens_repository;
use crate::lens_access::state::{LensDatabase, LensRoutesState, LensStorage};
use crate::lens_admin_routes::body::decode;
use crate::lens_admin_routes::error::{LensRouteError, route_key_scope};
use crate::lens_billing::key_validation::validate_key;

pub(crate) mod billing;

const TOKEN_BYTES: usize = 40;
const WORKER_NAME: &str = "Lens worker";

#[derive(Debug, Deserialize, Validate)]
#[serde(deny_unknown_fields)]
#[garde(allow_unvalidated)]
pub struct WorkerBilling {
    #[garde(pattern("^[a-f0-9]{64}$"))]
    pub(crate) analysis_key_id: String,
}

#[derive(Debug, Deserialize, Validate)]
#[serde(deny_unknown_fields)]
#[garde(allow_unvalidated)]
pub struct WorkerName {
    #[garde(pattern("^[a-f0-9]{64}$"))]
    pub(crate) analysis_key_id: String,
    #[serde(default = "default_name")]
    #[garde(length(chars, min = 1, max = 100))]
    pub(crate) name: String,
}

// the token is returned once on registration and stored only as its SHA-256 digest
#[derive(Serialize)]
pub(crate) struct RegisteredWorker {
    pub(crate) worker: Worker,
    pub(crate) token: String,
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
    Db: LensDatabase,
    Store: LensStorage,
{
    let request: WorkerName = decode(&body)?;
    let (_, scope) = route_key_scope::<Store>(caller, true)?;
    validate_key(state.keys.as_ref(), Some(&request.analysis_key_id)).await?;
    let encoded = URL_SAFE_NO_PAD.encode(state.ids.random_bytes(TOKEN_BYTES));
    let token = SecretString::from(format!("lens-{encoded}"));
    let worker = Worker {
        analysis_key_id: Some(request.analysis_key_id),
        id: state.ids.new_id(),
        name: request.name,
        scope,
        last_seen: DateTime::<Utc>::UNIX_EPOCH,
        revoked: false,
    };
    let repository = lens_repository(&state)?;
    let digest = hex::encode(Sha256::digest(token.expose_secret().as_bytes()));
    repository.save_worker(&worker, Some(&digest)).await?;
    Ok(Json(RegisteredWorker {
        worker,
        token: token.expose_secret().to_owned(),
    }))
}

pub(crate) async fn revoke_worker<Db, Store>(
    State(state): State<LensRoutesState<Db, Store>>,
    Path(worker_id): Path<String>,
    caller: Option<Extension<AuthContext>>,
) -> Result<Json<bool>, LensRouteError<<Store as Storage>::Error>>
where
    Db: LensDatabase,
    Store: LensStorage,
{
    let (repository, worker) = scoped_worker_for(&state, caller, &worker_id).await?;
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

// the caller's repository and the worker named by the path, once their scope covers it
async fn scoped_worker_for<'state, Db, Store>(
    state: &'state LensRoutesState<Db, Store>,
    caller: Option<Extension<AuthContext>>,
    worker_id: &str,
) -> Result<(&'state LensRepository<Db>, Worker), LensRouteError<<Store as Storage>::Error>>
where
    Db: LensDatabase,
    Store: LensStorage,
{
    let (_, scope) = route_key_scope::<Store>(caller, true)?;
    let repository = lens_repository(state)?;
    let workers = repository.workers().await?;
    let found = workers
        .into_iter()
        .find(|candidate| candidate.id == worker_id && can_access(&scope, &candidate.scope));
    let worker = found.ok_or(LensRouteError::Access(LensAccessError::WorkerNotFound))?;
    Ok((repository, worker))
}

mod tests;
