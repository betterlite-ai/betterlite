use std::collections::BTreeSet;
use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::extract::Extension;
use axum::extract::Path;
use axum::extract::Query;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::response::Response;
use axum::routing::get;
use betterlite_error::proxy_error_types::common::CommonProxyErrors;
use serde::Deserialize;
use serde::Serialize;
use thiserror::Error;

use crate::auth::auth_identity::AuthContext;
use crate::tracing_routes::router::unauthenticated;

use super::caller::CallerAccess;
use super::caller::access_of;
use super::config_read::config_endpoints;
use super::db_read::database_endpoints;
use super::ports::ConfigEndpointsSource;
use super::ports::GeneralSettingsReader;
use super::ports::TeamLookup;
use super::record::EndpointRecordError;
use super::record::PassThroughEndpointResponse;
use super::record::PassThroughGenericEndpoint;
use super::team_filter::TeamFilterError;
use super::team_filter::filter_endpoints_by_team;

const TEAM_NOT_FOUND: &str = "Team not found";

#[derive(Clone)]
pub struct PassThroughConfigState {
    pub settings: Option<Arc<dyn GeneralSettingsReader>>,
    pub config_endpoints: Arc<dyn ConfigEndpointsSource>,
    pub teams: Arc<dyn TeamLookup>,
}

pub fn routes(state: PassThroughConfigState) -> Router {
    Router::new()
        .route("/config/pass_through_endpoint", get(get_endpoints))
        .route(
            "/config/pass_through_endpoint/team/{team_id}",
            get(get_team_endpoints),
        )
        .with_state(state)
}

#[derive(Deserialize)]
struct EndpointQuery {
    endpoint_id: Option<String>,
    team_id: Option<String>,
}

#[derive(Deserialize)]
struct TeamEndpointQuery {
    endpoint_id: Option<String>,
}

async fn get_endpoints(
    State(state): State<PassThroughConfigState>,
    caller: Option<Extension<AuthContext>>,
    Query(query): Query<EndpointQuery>,
) -> Result<Json<PassThroughEndpointResponse>, PassThroughReadError> {
    let Some(Extension(context)) = caller else {
        return Err(PassThroughReadError::Unauthenticated);
    };
    serve(
        &state,
        access_of(&context),
        query.endpoint_id.as_deref(),
        query.team_id.as_deref(),
    )
    .await
}

async fn get_team_endpoints(
    State(state): State<PassThroughConfigState>,
    caller: Option<Extension<AuthContext>>,
    Path(team_id): Path<String>,
    Query(query): Query<TeamEndpointQuery>,
) -> Result<Json<PassThroughEndpointResponse>, PassThroughReadError> {
    let Some(Extension(context)) = caller else {
        return Err(PassThroughReadError::Unauthenticated);
    };
    serve(
        &state,
        access_of(&context),
        query.endpoint_id.as_deref(),
        Some(&team_id),
    )
    .await
}

// both routes share it: the database wins over the config file on a path, and an endpoint_id narrows the answer to the database
async fn serve(
    state: &PassThroughConfigState,
    access: CallerAccess,
    endpoint_id: Option<&str>,
    team_id: Option<&str>,
) -> Result<Json<PassThroughEndpointResponse>, PassThroughReadError> {
    let Some(settings) = state.settings.as_deref() else {
        return Err(PassThroughReadError::DbNotConnected);
    };
    let database = database_endpoints(settings, endpoint_id, Some(access))
        .await
        .map_err(ReadFailure::Endpoint)?;
    let config = config_endpoints(state.config_endpoints.as_ref());
    let mut endpoints = merged_endpoints(config, database, endpoint_id);
    if let Some(team_id) = team_id {
        match filter_endpoints_by_team(state.teams.as_ref(), team_id, &endpoints).await {
            Ok(allowed) => endpoints = allowed,
            Err(TeamFilterError::TeamNotFound) => return Err(PassThroughReadError::TeamNotFound),
            Err(error) => return Err(PassThroughReadError::Internal(ReadFailure::Team(error))),
        }
    }
    Ok(Json(PassThroughEndpointResponse { endpoints }))
}

fn merged_endpoints(
    config: Vec<PassThroughGenericEndpoint>,
    database: Vec<PassThroughGenericEndpoint>,
    endpoint_id: Option<&str>,
) -> Vec<PassThroughGenericEndpoint> {
    if endpoint_id.is_some() {
        return database;
    }
    let database_paths: BTreeSet<&str> = database
        .iter()
        .map(|endpoint| endpoint.path.as_str())
        .collect();
    let mut merged: Vec<PassThroughGenericEndpoint> = config
        .into_iter()
        .filter(|endpoint| !database_paths.contains(endpoint.path.as_str()))
        .collect();
    merged.extend(database);
    merged
}

#[derive(Debug, Error)]
pub enum PassThroughReadError {
    #[error("the API key is not valid")]
    Unauthenticated,
    #[error("the database is not connected")]
    DbNotConnected,
    #[error("the team was not found")]
    TeamNotFound,
    #[error("the pass-through endpoint read failed")]
    Internal(#[from] ReadFailure),
}

#[derive(Debug, Error)]
pub enum ReadFailure {
    #[error(transparent)]
    Endpoint(#[from] EndpointRecordError),
    #[error(transparent)]
    Team(#[from] TeamFilterError),
}

#[derive(Serialize)]
struct DetailBody {
    detail: DetailError,
}

#[derive(Serialize)]
struct DetailError {
    error: &'static str,
}

#[derive(Serialize)]
struct InternalBody {
    error: InternalMessage,
}

#[derive(Serialize)]
struct InternalMessage {
    message: &'static str,
    #[serde(rename = "type")]
    error_type: &'static str,
}

impl IntoResponse for PassThroughReadError {
    fn into_response(self) -> Response {
        match self {
            Self::Unauthenticated => unauthenticated(),
            Self::DbNotConnected => {
                let body = DetailBody {
                    detail: DetailError {
                        error: CommonProxyErrors::DbNotConnectedError.as_str(),
                    },
                };
                (StatusCode::INTERNAL_SERVER_ERROR, Json(body)).into_response()
            }
            Self::TeamNotFound => {
                let body = DetailBody {
                    detail: DetailError {
                        error: TEAM_NOT_FOUND,
                    },
                };
                (StatusCode::NOT_FOUND, Json(body)).into_response()
            }
            Self::Internal(error) => {
                tracing::error!(error = ?error, "the pass-through endpoint read failed");
                let body = InternalBody {
                    error: InternalMessage {
                        message: "Internal server error",
                        error_type: "internal_server_error",
                    },
                };
                (StatusCode::INTERNAL_SERVER_ERROR, Json(body)).into_response()
            }
        }
    }
}
