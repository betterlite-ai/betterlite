use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::extract::{Extension, Path, RawQuery, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse as _, Response};
use axum::routing::get;
use chrono::Utc;
use serde::Serialize;

use crate::auth::auth_identity::AuthContext;
use crate::plugin_routes::routes::{Identity, caller_identity};
use crate::shadow_eval_results::access::require_viewer;
use crate::shadow_eval_results::error::ShadowResultsRouteError;
use crate::shadow_eval_results::postgres::store_from_state;
use crate::shadow_eval_results::query::decode_shadow_query;
use crate::shadow_eval_results::service::{get_shadow_eval_job, list_shadow_eval_jobs};
use crate::shadow_result_aggregation::postgres::PgShadowResults;
use crate::state::ServerState;
use crate::tracing_routes::router::unauthenticated;

const VIEW_ACTION: &str = "view shadow evals";

// what the listing and detail reads open, resolved once when the router is built
struct ResultsState {
    store: Option<Arc<PgShadowResults>>,
}

#[derive(Serialize)]
struct DetailBody {
    detail: String,
}

#[cfg_attr(test, mutants::skip)]
pub(crate) fn routes(state: &Arc<ServerState>) -> Router {
    let results = Arc::new(ResultsState {
        store: store_from_state(state).map(Arc::new),
    });
    Router::new()
        .route("/auto_router/shadow_eval", get(list_route))
        .route("/auto_router/shadow_eval/{job_id}", get(detail_route))
        .with_state(results)
}

// FastAPI checks the auth (401) before it validates the query (422); the endpoint checks the role (403)
async fn list_route(
    State(state): State<Arc<ResultsState>>,
    caller: Option<Extension<AuthContext>>,
    RawQuery(raw_query): RawQuery,
) -> Result<Response, Response> {
    let identity = identity_of_caller(caller.as_ref())?;
    let query = decode_shadow_query(raw_query.as_deref().unwrap_or_default().as_bytes())
        .map_err(|error| failure(&ShadowResultsRouteError::from(error)))?;
    let store = require_viewer(&identity, VIEW_ACTION, state.store.as_deref())
        .map_err(|error| failure(&error))?;
    let jobs = list_shadow_eval_jobs(store, &query, Utc::now())
        .await
        .map_err(|error| failure(&error))?;
    Ok(Json(jobs).into_response())
}

async fn detail_route(
    State(state): State<Arc<ResultsState>>,
    Path(job_id): Path<String>,
    caller: Option<Extension<AuthContext>>,
) -> Result<Response, Response> {
    let identity = identity_of_caller(caller.as_ref())?;
    let store = require_viewer(&identity, VIEW_ACTION, state.store.as_deref())
        .map_err(|error| failure(&error))?;
    match get_shadow_eval_job(store, &job_id, Utc::now()).await {
        Ok(job) => Ok(Json(job).into_response()),
        Err(error) => Err(failure(&error)),
    }
}

fn identity_of_caller(caller: Option<&Extension<AuthContext>>) -> Result<Identity, Response> {
    let Some(extension) = caller else {
        return Err(unauthenticated());
    };
    caller_identity(&extension.principal).map_err(|error| {
        let detail = format!("the caller's identity could not be read: {error}");
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(DetailBody { detail }),
        )
            .into_response()
    })
}

fn failure(error: &ShadowResultsRouteError) -> Response {
    let status = match error.status_code() {
        400 => StatusCode::BAD_REQUEST,
        403 => StatusCode::FORBIDDEN,
        404 => StatusCode::NOT_FOUND,
        422 => StatusCode::UNPROCESSABLE_ENTITY,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    };
    (
        status,
        Json(DetailBody {
            detail: error.to_string(),
        }),
    )
        .into_response()
}

mod tests;
