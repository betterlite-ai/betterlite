use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::extract::{Extension, RawQuery, State};
use axum::http::{StatusCode, Uri};
use axum::response::{IntoResponse as _, Response};
use axum::routing::get;
use chrono::Utc;
use serde::Serialize;

use crate::auth::auth_identity::AuthContext;
use crate::auto_router_read_api::access::caller_api_key;
use crate::auto_router_read_api::error::ReadApiError;
use crate::auto_router_read_api::postgres::reads_from_state;
use crate::auto_router_read_api::query::{
    ReadQueryError, decode_benchmarks_query, decode_session_query,
};
use crate::auto_router_read_api::service::{benchmarks_for, session_for};
use crate::auto_router_read_api::store::AutoRouterReadStore;
use crate::auto_router_read_api::{BENCHMARKS_PATH, SESSION_PATH, VIEW_ACTION};
use crate::plugin_routes::routes::caller_identity;
use crate::routing::auto_router_read_api::{ConfiguredRouter, configured_deployments};
use crate::shadow_eval_lifecycle::access::require_admin;
use crate::shadow_eval_validation::access::AdminRole;
use crate::state::ServerState;
use crate::tracing_routes::router::{ValidationBody, ValidationIssue, unauthenticated};

// what the two reads open, resolved once when the router is built
struct ReadState {
    store: Option<Arc<dyn AutoRouterReadStore>>,
    deployments: Vec<ConfiguredRouter>,
}

#[derive(Serialize)]
struct DetailBody {
    detail: String,
}

#[cfg_attr(test, mutants::skip)]
pub(crate) fn routes(state: &Arc<ServerState>) -> Router {
    let reads = Arc::new(ReadState {
        store: reads_from_state(state),
        deployments: configured_deployments(&state.config.model_list),
    });
    Router::new()
        .route(BENCHMARKS_PATH, get(read))
        .route(SESSION_PATH, get(read))
        .with_state(reads)
}

// FastAPI checks the auth (401), then binds the query (422), then runs the endpoint body (403, 500)
async fn read(
    State(state): State<Arc<ReadState>>,
    caller: Option<Extension<AuthContext>>,
    RawQuery(raw_query): RawQuery,
    uri: Uri,
) -> Response {
    let Some(extension) = caller else {
        return unauthenticated();
    };
    let raw = raw_query.as_deref().unwrap_or_default();
    if uri.path() == BENCHMARKS_PATH {
        return benchmarks_read(&state, &extension, raw).await;
    }
    let query = match decode_session_query(raw.as_bytes()) {
        Ok(query) => query,
        Err(error) => return failure(&ReadApiError::from(error)),
    };
    let Some(store) = state.store.as_deref() else {
        return failure(&ReadApiError::DatabaseUnavailable);
    };
    let api_key = match caller_api_key(&extension.principal) {
        Ok(api_key) => api_key,
        Err(source) => return failure(&ReadApiError::CallerKey { source }),
    };
    match session_for(store, api_key.as_deref(), &query).await {
        Ok(response) => Json(response).into_response(),
        Err(error) => failure(&error),
    }
}

// the benchmarks read needs the key's identity and role before the database and the window
async fn benchmarks_read(state: &ReadState, extension: &AuthContext, raw: &str) -> Response {
    let identity = match caller_identity(&extension.principal) {
        Ok(identity) => identity,
        Err(source) => return failure(&ReadApiError::Identity { source }),
    };
    let query = match decode_benchmarks_query(raw.as_bytes()) {
        Ok(query) => query,
        Err(error) => return failure(&ReadApiError::from(error)),
    };
    if let Err(error) = require_admin(&identity, VIEW_ACTION, AdminRole::Viewer) {
        return failure(&ReadApiError::from(error));
    }
    let Some(store) = state.store.as_deref() else {
        return failure(&ReadApiError::DatabaseUnavailable);
    };
    match benchmarks_for(store, &query, &state.deployments, Utc::now().date_naive()).await {
        Ok(benchmarks) => Json(benchmarks).into_response(),
        Err(error) => failure(&error),
    }
}

// the shared failure body: a Query rejection is the validation envelope, every other failure a detail
fn failure(error: &ReadApiError) -> Response {
    if let ReadApiError::Query(ReadQueryError::Invalid {
        parameter,
        kind,
        msg,
    }) = error
    {
        let body = ValidationBody {
            detail: vec![ValidationIssue {
                kind,
                loc: vec!["query".to_owned(), (*parameter).to_owned()],
                msg,
            }],
        };
        return (StatusCode::UNPROCESSABLE_ENTITY, Json(body)).into_response();
    }
    let status = match error.status_code() {
        400 => StatusCode::BAD_REQUEST,
        403 => StatusCode::FORBIDDEN,
        404 => StatusCode::NOT_FOUND,
        422 => StatusCode::UNPROCESSABLE_ENTITY,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    };
    let body = DetailBody {
        detail: error.to_string(),
    };
    (status, Json(body)).into_response()
}

mod tests;
