use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::extract::{Extension, OriginalUri, Path, RawQuery, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use chrono::Utc;
use serde::Serialize;

use crate::auth::auth_identity::AuthContext;
use crate::plugin_routes::routes::{Identity, caller_identity};
use crate::shadow_eval_results::access::require_viewer;
use crate::shadow_eval_results::error::ShadowResultsRouteError;
use crate::shadow_eval_results::postgres::store_from_state;
use crate::shadow_eval_results::query::{ShadowQueryError, decode_shadow_query};
use crate::shadow_eval_results::service::{get_shadow_eval_job, list_shadow_eval_jobs};
use crate::shadow_eval_results::{DETAIL_PATH, LIST_PATH, START_PATH};
use crate::shadow_result_aggregation::postgres::PgShadowResults;
use crate::state::ServerState;
use crate::tracing_routes::router::{ValidationBody, ValidationIssue, unauthenticated};

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
        .route(LIST_PATH, get(shadow_eval_read))
        .route(DETAIL_PATH, get(shadow_eval_read))
        .route(START_PATH, get(shadow_eval_read))
        .with_state(results)
}

// the handlers' failure path; the box keeps their Err one pointer wide for the lint table
struct RouteFailure(Box<Response>);

impl From<Response> for RouteFailure {
    fn from(response: Response) -> Self {
        Self(Box::new(response))
    }
}

impl IntoResponse for RouteFailure {
    fn into_response(self) -> Response {
        *self.0
    }
}

// FastAPI checks the auth (401) before it validates the query (422), then the endpoint checks the role (403)
async fn shadow_eval_read(
    State(state): State<Arc<ResultsState>>,
    OriginalUri(uri): OriginalUri,
    job: Option<Path<String>>,
    caller: Option<Extension<AuthContext>>,
    RawQuery(raw_query): RawQuery,
) -> Result<Response, RouteFailure> {
    let identity = identity_of_caller(caller.as_ref())?;
    let job_id = match job {
        Some(Path(job_id)) => job_id,
        // LiteLLM's {job_id} route reads GET on the static start path as job "start"
        None if uri.path() == START_PATH => "start".to_owned(),
        None => {
            let query = decode_shadow_query(raw_query.as_deref().unwrap_or_default().as_bytes())
                .map_err(|error| {
                    RouteFailure::from(failure(&ShadowResultsRouteError::from(error)))
                })?;
            let store = viewer_store(&state, &identity)?;
            return answered(list_shadow_eval_jobs(store, &query, Utc::now()).await);
        }
    };
    let store = viewer_store(&state, &identity)?;
    answered(get_shadow_eval_job(store, &job_id, Utc::now()).await)
}

fn identity_of_caller(caller: Option<&Extension<AuthContext>>) -> Result<Identity, RouteFailure> {
    let Some(extension) = caller else {
        return Err(RouteFailure::from(unauthenticated()));
    };
    caller_identity(&extension.principal).map_err(|error| {
        let detail = format!("the caller's identity could not be read: {error}");
        RouteFailure::from(
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(DetailBody { detail }),
            )
                .into_response(),
        )
    })
}

// the one read each route serves, answered with the shared failure body
fn answered<T>(read: Result<T, ShadowResultsRouteError>) -> Result<Response, RouteFailure>
where
    T: Serialize,
{
    match read {
        Ok(value) => Ok(Json(value).into_response()),
        Err(error) => Err(RouteFailure::from(failure(&error))),
    }
}

// the viewer role every read checks, after the caller's identity
fn viewer_store<'state>(
    state: &'state ResultsState,
    identity: &Identity,
) -> Result<&'state PgShadowResults, RouteFailure> {
    require_viewer(identity, VIEW_ACTION, state.store.as_deref())
        .map_err(|error| RouteFailure::from(failure(&error)))
}

fn failure(error: &ShadowResultsRouteError) -> Response {
    if let ShadowResultsRouteError::Query(ShadowQueryError::Invalid {
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
    (
        status,
        Json(DetailBody {
            detail: error.to_string(),
        }),
    )
        .into_response()
}

mod tests;
