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
use crate::shadow_eval_results::postgres::{PgShadowResults, store_from_state};
use crate::shadow_eval_results::query::decode_shadow_query;
use crate::shadow_eval_results::service::{get_shadow_eval_job, list_shadow_eval_jobs};
use crate::state::ServerState;
use crate::tracing_routes::router::unauthenticated;

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

// FastAPI validates the query before the endpoint runs and checks the role before the database
async fn list_route(
    State(state): State<Arc<ResultsState>>,
    caller: Option<Extension<AuthContext>>,
    RawQuery(raw_query): RawQuery,
) -> Response {
    let identity = match identity_of_caller(caller.as_ref()) {
        Ok(identity) => identity,
        Err(response) => return *response,
    };
    let query = match decode_shadow_query(raw_query.as_deref().unwrap_or_default().as_bytes()) {
        Ok(query) => query,
        Err(error) => return failure(&ShadowResultsRouteError::from(error)),
    };
    let store = match require_viewer(&identity, "view shadow evals", state.store.as_deref()) {
        Ok(store) => store,
        Err(error) => return failure(&error),
    };
    match list_shadow_eval_jobs(store, &query, Utc::now()).await {
        Ok(jobs) => Json(jobs).into_response(),
        Err(error) => failure(&error),
    }
}

async fn detail_route(
    State(state): State<Arc<ResultsState>>,
    Path(job_id): Path<String>,
    caller: Option<Extension<AuthContext>>,
) -> Response {
    let identity = match identity_of_caller(caller.as_ref()) {
        Ok(identity) => identity,
        Err(response) => return *response,
    };
    let store = match require_viewer(&identity, "view shadow evals", state.store.as_deref()) {
        Ok(store) => store,
        Err(error) => return failure(&error),
    };
    match get_shadow_eval_job(store, &job_id, Utc::now()).await {
        Ok(job) => Json(job).into_response(),
        Err(error) => failure(&error),
    }
}

fn identity_of_caller(caller: Option<&Extension<AuthContext>>) -> Result<Identity, Box<Response>> {
    let Some(extension) = caller else {
        return Err(Box::new(unauthenticated()));
    };
    caller_identity(&extension.principal).map_err(|error| {
        let detail = format!("the caller's identity could not be read: {error}");
        Box::new(
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(DetailBody { detail }),
            )
                .into_response(),
        )
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
