use axum::http::HeaderValue;
use betterlite_lens::models::job::Worker;
use sha2::{Digest as _, Sha256};

use crate::lens_access::lookup::lens_repository;
use crate::lens_access::state::{LensDatabase, LensRoutesState};
use crate::lens_worker_routes::error::LensWorkerError;

// worker_auth: HTTPBearer's parse, then the SHA-256 of the token looked up in LiteLLM_LensWorker
pub(crate) async fn worker_auth<Db, Store>(
    state: &LensRoutesState<Db, Store>,
    header: Option<&HeaderValue>,
) -> Result<Worker, LensWorkerError>
where
    Db: LensDatabase,
{
    let token = bearer_token(header)?;
    let worker = lens_repository(state)?.worker(&sha256_hex(&token)).await?;
    match worker {
        Some(found) if !found.revoked => Ok(found),
        Some(_) | None => Err(LensWorkerError::InvalidCredential),
    }
}

// FastAPI 0.136.3: partition on the first space, strip the credentials, compare the scheme ignoring case
fn bearer_token(header: Option<&HeaderValue>) -> Result<String, LensWorkerError> {
    let value = header
        .and_then(|value| value.to_str().ok())
        .filter(|value| !value.is_empty())
        .ok_or(LensWorkerError::NotAuthenticated)?;
    let (scheme, rest) = value.split_once(' ').unwrap_or((value, ""));
    let credentials = rest.trim();
    if scheme.is_empty() || credentials.is_empty() || !scheme.eq_ignore_ascii_case("bearer") {
        return Err(LensWorkerError::NotAuthenticated);
    }
    Ok(credentials.to_owned())
}

fn sha256_hex(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

mod tests;
