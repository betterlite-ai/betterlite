use axum::Json;
use axum::body::Bytes;
use axum::extract::{Extension, Path, State};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use betterlite_db::lens_repository::repository::LensRepository;
use betterlite_lens::models::job::{JobStatus, Worker};
use betterlite_lens::models::settings::Scope;
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
use crate::lens_access::state::{LensDatabase, LensRoutesState, LensStorage};
use crate::lens_admin_routes::body::decode;
use crate::lens_admin_routes::error::{LensRouteError, route_key_scope};
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

impl<Db, Store> LensRoutesState<Db, Store>
where
    Db: LensDatabase,
    Store: LensStorage,
{
    pub(crate) async fn register_worker(
        State(state): State<Self>,
        caller: Option<Extension<AuthContext>>,
        body: Bytes,
    ) -> Result<Json<RegisteredWorker>, LensRouteError<<Store as Storage>::Error>> {
        Ok(Json(state.register_from(caller, body).await?))
    }

    pub(crate) async fn set_worker_billing(
        State(state): State<Self>,
        Path(worker_id): Path<String>,
        caller: Option<Extension<AuthContext>>,
        body: Bytes,
    ) -> Result<Json<Worker>, LensRouteError<<Store as Storage>::Error>> {
        Ok(Json(state.billing_for(caller, worker_id, body).await?))
    }

    pub(crate) async fn revoke_worker(
        State(state): State<Self>,
        Path(worker_id): Path<String>,
        caller: Option<Extension<AuthContext>>,
    ) -> Result<Json<bool>, LensRouteError<<Store as Storage>::Error>> {
        Ok(Json(state.revoke_by(caller, worker_id).await?))
    }

    async fn register_from(
        &self,
        caller: Option<Extension<AuthContext>>,
        body: Bytes,
    ) -> Result<RegisteredWorker, LensRouteError<<Store as Storage>::Error>> {
        let request: WorkerName = decode(&body)?;
        let (_, scope) = route_key_scope::<Store>(caller, true)?;
        validate_key(self.keys.as_ref(), Some(&request.analysis_key_id)).await?;
        let encoded = URL_SAFE_NO_PAD.encode(self.ids.random_bytes(TOKEN_BYTES));
        let token = format!("lens-{encoded}");
        let worker = Worker {
            analysis_key_id: Some(request.analysis_key_id.clone()),
            id: self.ids.new_id(),
            name: request.name,
            scope,
            last_seen: DateTime::<Utc>::UNIX_EPOCH,
            revoked: false,
        };
        let digest = hex::encode(Sha256::digest(token.as_bytes()));
        let repository = lens_repository(self)?;
        repository.save_worker(&worker, Some(&digest)).await?;
        Ok(RegisteredWorker {
            worker,
            token: SecretString::from(token),
        })
    }

    async fn billing_for(
        &self,
        caller: Option<Extension<AuthContext>>,
        worker_id: String,
        body: Bytes,
    ) -> Result<Worker, LensRouteError<<Store as Storage>::Error>> {
        let request: WorkerBilling = decode(&body)?;
        let (_, scope) = route_key_scope::<Store>(caller, true)?;
        let repository = lens_repository(self)?;
        let worker = scoped_worker(repository, &scope, &worker_id).await?;
        if worker.revoked {
            return Err(LensRouteError::Access(LensAccessError::WorkerRevoked));
        }
        validate_key(self.keys.as_ref(), Some(&request.analysis_key_id)).await?;
        let updated = repository
            .set_worker_billing(&worker.id, &request.analysis_key_id)
            .await?;
        let Some(updated) = updated else {
            return Err(LensRouteError::Access(LensAccessError::WorkerAccessRevoked));
        };
        Ok(updated)
    }

    async fn revoke_by(
        &self,
        caller: Option<Extension<AuthContext>>,
        worker_id: String,
    ) -> Result<bool, LensRouteError<<Store as Storage>::Error>> {
        let (_, scope) = route_key_scope::<Store>(caller, true)?;
        let repository = lens_repository(self)?;
        let worker = scoped_worker(repository, &scope, &worker_id).await?;
        let lenses = repository.lenses().await?;
        let busy = lenses.iter().flat_map(|lens| lens.jobs.iter()).any(|job| {
            job.status == JobStatus::Running && job.worker_id.as_deref() == Some(worker.id.as_str())
        });
        if busy {
            return Err(LensRouteError::Access(LensAccessError::WorkerRunning));
        }
        repository.revoke_worker(&worker.id).await?;
        Ok(true)
    }
}

// the worker named by the path, when the caller's scope covers it, else LiteLLM's 404
async fn scoped_worker<Db, Store>(
    repository: &LensRepository<Db>,
    scope: &Scope,
    worker_id: &str,
) -> Result<Worker, LensRouteError<<Store as Storage>::Error>>
where
    Db: LensDatabase,
    Store: Storage,
{
    let workers = repository.workers().await?;
    let found = workers
        .into_iter()
        .find(|worker| worker.id == worker_id && can_access(scope, &worker.scope));
    match found {
        Some(worker) => Ok(worker),
        None => Err(LensRouteError::Access(LensAccessError::WorkerNotFound)),
    }
}

mod tests;
