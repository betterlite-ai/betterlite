use axum::http::HeaderValue;
use betterlite_lens::models::job::Worker;
use betterlite_lens::sources::storage::Storage;
use secrecy::{ExposeSecret as _, SecretString};
use sha2::{Digest as _, Sha256};

use crate::lens_access::lookup::lens_repository;
use crate::lens_access::state::{LensDatabase, LensRoutesState, LensStorage};
use crate::lens_worker_routes::error::LensWorkerError;

// worker_auth: HTTPBearer's parse, then the SHA-256 of the token looked up in LiteLLM_LensWorker
pub(crate) async fn worker_auth<Db, Store>(
    state: &LensRoutesState<Db, Store>,
    header: Option<&HeaderValue>,
) -> Result<Worker, LensWorkerError<<Store as Storage>::Error>>
where
    Db: LensDatabase,
    Store: LensStorage,
{
    let token = bearer_token(header).ok_or(LensWorkerError::NotAuthenticated)?;
    let worker = lens_repository(state)?
        .worker(&hex::encode(Sha256::digest(
            token.expose_secret().as_bytes(),
        )))
        .await?;
    match worker {
        Some(found) if !found.revoked => Ok(found),
        Some(_) | None => Err(LensWorkerError::InvalidCredential),
    }
}

// FastAPI 0.136.3: partition on the first space, strip the credentials, compare the scheme ignoring case
pub fn bearer_token(header: Option<&HeaderValue>) -> Option<SecretString> {
    let value = header
        .and_then(|value| value.to_str().ok())
        .filter(|value| !value.is_empty())?;
    let (scheme, rest) = value.split_once(' ').unwrap_or((value, ""));
    let credentials = rest.trim();
    if scheme.is_empty() || credentials.is_empty() || !scheme.eq_ignore_ascii_case("bearer") {
        return None;
    }
    Some(SecretString::from(credentials.to_owned()))
}

mod tests;
