use axum::Json;
use axum::extract::{Path, RawQuery, State};
use axum::http::{HeaderMap, header};
use betterlite_lens::models::execution::ExecutionContent;
use betterlite_lens::sources::storage::Storage;

use crate::lens_access::state::{LensDatabase, LensStorage};
use crate::lens_access::storage::source_reader;
use crate::lens_admin_routes::body::decode_content_query;
use crate::lens_worker_routes::WorkerRoutesState;
use crate::lens_worker_routes::credential::worker_auth;
use crate::lens_worker_routes::error::{LensWorkerError, query_refusal};
use crate::lens_worker_routes::lease::assigned;

pub(crate) async fn content<Db, Store>(
    State(state): State<WorkerRoutesState<Db, Store>>,
    Path((lens_id, job_id)): Path<(String, String)>,
    headers: HeaderMap,
    raw: RawQuery,
) -> Result<Json<ExecutionContent>, LensWorkerError<<Store as Storage>::Error>>
where
    Db: LensDatabase,
    Store: LensStorage,
{
    let worker = worker_auth(&state.routes, headers.get(header::AUTHORIZATION)).await?;
    let query_text = raw.0.as_deref().unwrap_or_default();
    let execution_id = execution_id(query_text.as_bytes())
        .ok_or_else(|| query_refusal("missing", "execution_id", "Field required"))?;
    let query = decode_content_query(query_text.as_bytes())?;
    let now = state.routes.clock.now();
    let (lens, job) = assigned(&state.routes, &lens_id, &job_id, &worker, now).await?;
    let execution = job
        .sample
        .as_ref()
        .and_then(|sample| {
            sample
                .executions
                .iter()
                .find(|entry| entry.id == execution_id)
        })
        .ok_or(LensWorkerError::OutsideSample)?;
    let reader = source_reader(state.routes.storage.as_ref())?;
    let body = reader
        .content(&lens.scope, execution, &query.cursor, query.offset)
        .await?;
    Ok(Json(body))
}

// execution_id is a required query string, so a request without it is FastAPI's missing-field 422
#[must_use]
pub fn execution_id(raw: &[u8]) -> Option<String> {
    let mut found = None;
    for (name, value) in form_urlencoded::parse(raw).into_owned() {
        if name == "execution_id" {
            found = Some(value);
        }
    }
    found
}

mod tests;
