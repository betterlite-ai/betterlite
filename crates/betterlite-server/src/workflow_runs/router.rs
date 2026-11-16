use axum::body::Bytes;
use axum::extract::{Extension, Path, Query, State};
use axum::http::Uri;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde::de::DeserializeOwned;

use crate::auth::auth_identity::AuthContext;
use crate::workflow_runs::WorkflowRunsState;
use crate::workflow_runs::access::{RunCaller, ensure_owner, require_run};
use crate::workflow_runs::error::{RunsError, ValidationFailure};
use crate::workflow_runs::models::{
    RunCreateData, RunListResponse, RunUpdateData, RunWhere, StatusFilter,
    WorkflowRunCreateRequest, WorkflowRunRow, WorkflowRunUpdateRequest,
};
use crate::workflow_runs::store::WorkflowRunStore;

const DEFAULT_LIMIT: u8 = 50;
const MAX_LIMIT: u8 = 250;

#[derive(Debug, Deserialize)]
struct ListQuery {
    workflow_type: Option<String>,
    status: Option<String>,
    limit: Option<String>,
}

pub fn router(state: WorkflowRunsState) -> Router {
    Router::new()
        .route("/v1/workflows/runs", post(create_run).get(list_runs))
        .route(
            "/v1/workflows/runs/{run_id}",
            get(get_run).patch(update_run),
        )
        .with_state(state)
}

pub fn decode_body<Request>(body: &[u8]) -> Result<Request, ValidationFailure>
where
    Request: DeserializeOwned,
{
    let mut deserializer = serde_json::Deserializer::from_slice(body);
    let decoded = serde_path_to_error::deserialize(&mut deserializer).map_err(|source| {
        ValidationFailure::Body {
            source: Box::new(source),
        }
    })?;
    deserializer
        .end()
        .map_err(|source| ValidationFailure::Trailing { source })?;
    Ok(decoded)
}

fn require_caller(identity: Option<Extension<AuthContext>>) -> Result<RunCaller, RunsError> {
    let Some(Extension(context)) = identity else {
        return Err(RunsError::Unauthenticated);
    };
    Ok(RunCaller::from_principal(&context.principal))
}

fn require_store(state: &WorkflowRunsState) -> Result<&dyn WorkflowRunStore, RunsError> {
    let Some(store) = state.store.as_deref() else {
        return Err(RunsError::DbNotConnected);
    };
    Ok(store)
}

fn parse_limit(raw: Option<&str>) -> Result<u8, ValidationFailure> {
    let Some(text) = raw else {
        return Ok(DEFAULT_LIMIT);
    };
    let limit: i128 = text
        .parse()
        .map_err(|_error| ValidationFailure::LimitNotInteger)?;
    match limit {
        ..=0 => Err(ValidationFailure::LimitTooSmall),
        1.. => u8::try_from(limit)
            .ok()
            .filter(|bounded| *bounded <= MAX_LIMIT)
            .ok_or(ValidationFailure::LimitTooLarge),
    }
}

// "running,paused" is an in-clause; a single value stays a plain equality
fn status_filter(text: &str) -> StatusFilter {
    let statuses: Vec<String> = text.split(',').map(|part| part.trim().to_owned()).collect();
    match <[String; 1]>::try_from(statuses) {
        Ok([single]) => StatusFilter::Equals(single),
        Err(several) => StatusFilter::In(several),
    }
}

async fn create_run(
    State(state): State<WorkflowRunsState>,
    identity: Option<Extension<AuthContext>>,
    body: Bytes,
) -> Result<Json<WorkflowRunRow>, RunsError> {
    let caller = require_caller(identity)?;
    let request: WorkflowRunCreateRequest = decode_body(&body)?;
    let store = require_store(&state)?;
    let data = RunCreateData::from_request(&request, caller.key)?;
    Ok(Json(store.create(&data).await?))
}

async fn list_runs(
    State(state): State<WorkflowRunsState>,
    identity: Option<Extension<AuthContext>>,
    uri: Uri,
) -> Result<Json<RunListResponse>, RunsError> {
    let caller = require_caller(identity)?;
    let Ok(Query(query)) = Query::<ListQuery>::try_from_uri(&uri) else {
        return Err(RunsError::from(ValidationFailure::Query));
    };
    let take = parse_limit(query.limit.as_deref())?;
    let store = require_store(&state)?;
    // LiteLLM lists every run for a non-admin caller that has no key token; that caller sees none here
    let created_by = match (caller.has_admin_view(), caller.key) {
        (true, _) => None,
        (false, Some(key)) => Some(key),
        (false, None) => {
            return Ok(Json(RunListResponse {
                runs: Vec::new(),
                count: 0,
            }));
        }
    };
    let filter = RunWhere {
        workflow_type: query.workflow_type.filter(|text| !text.is_empty()),
        status: query
            .status
            .filter(|text| !text.is_empty())
            .map(|text| status_filter(&text)),
        created_by,
    };
    let runs = store.find_many(&filter, take).await?;
    let count = runs.len();
    Ok(Json(RunListResponse { runs, count }))
}

async fn get_run(
    State(state): State<WorkflowRunsState>,
    identity: Option<Extension<AuthContext>>,
    Path(run_id): Path<String>,
) -> Result<Json<WorkflowRunRow>, RunsError> {
    let caller = require_caller(identity)?;
    let store = require_store(&state)?;
    let run = store
        .find_unique_with_latest_event(&run_id)
        .await?
        .ok_or(RunsError::NotFound { run_id })?;
    Ok(Json(ensure_owner(run, caller.read_scope())?))
}

async fn update_run(
    State(state): State<WorkflowRunsState>,
    identity: Option<Extension<AuthContext>>,
    Path(run_id): Path<String>,
    body: Bytes,
) -> Result<Json<WorkflowRunRow>, RunsError> {
    let caller = require_caller(identity)?;
    let request: WorkflowRunUpdateRequest = decode_body(&body)?;
    let store = require_store(&state)?;
    let update = RunUpdateData::from_request(&request)?;
    if update == RunUpdateData::default() {
        return Err(RunsError::NoFieldsToUpdate);
    }
    require_run(store, &run_id, Some(&caller)).await?;
    let run = store
        .update(&run_id, &update)
        .await?
        .ok_or(RunsError::NotFound { run_id })?;
    Ok(Json(run))
}
