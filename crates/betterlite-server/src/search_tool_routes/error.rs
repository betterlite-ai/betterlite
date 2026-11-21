use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use betterlite_auth::auth_team_lookup::error::{Detail, TeamLookupError};
use betterlite_db::search_tools::error::SearchToolsError;
use serde::Serialize;
use thiserror::Error;

use super::body::BodyError;
use super::list::ListError;
use crate::tracing_routes::router::unauthenticated;

#[derive(Debug, Error)]
pub(super) enum RouteError {
    #[error("the request has no authenticated caller")]
    Unauthenticated,
    #[error("{0}")]
    Body(#[source] BodyError),
    // raised before LiteLLM's try block, so it is never relabeled
    #[error("Prisma client not initialized")]
    NoDatabase,
    #[error("Search tool with ID {search_tool_id} not found")]
    NotFound { search_tool_id: String },
    #[error("{0}")]
    Registry(#[source] Box<SearchToolsError>),
    #[error("{0}")]
    List(#[from] ListError),
}

impl From<SearchToolsError> for RouteError {
    fn from(error: SearchToolsError) -> Self {
        Self::Registry(Box::new(error))
    }
}

#[derive(Serialize)]
struct DetailBody {
    detail: DetailValue,
}

#[derive(Serialize)]
#[serde(untagged)]
enum DetailValue {
    Text(String),
    Error { error: String },
}

// LiteLLM's verbose_proxy_logger.exception(<context>: %s, e) before the 500 that carries str(e)
pub(super) fn logged<Failure>(context: &str, failure: Failure) -> RouteError
where
    RouteError: From<Failure>,
{
    let error = RouteError::from(failure);
    tracing::error!("{context}: {error}");
    error
}

// the list re-raises an HTTPException from the team lookup as it is, and logs every other failure
pub(super) fn list_failure(error: ListError) -> RouteError {
    if let ListError::Team(team) = &error
        && team.http().is_some()
    {
        return RouteError::List(error);
    }
    logged("Error getting search tools", error)
}

fn team_failure(team: &TeamLookupError) -> Response {
    let message = team.to_string();
    let (code, detail) = match team.http() {
        Some((status_code, Detail::Error)) => (status_code, DetailValue::Error { error: message }),
        Some((status_code, Detail::Text)) => (status_code, DetailValue::Text(message)),
        None => (500_u16, DetailValue::Text(message)),
    };
    let status = StatusCode::from_u16(code).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    (status, Json(DetailBody { detail })).into_response()
}

impl IntoResponse for RouteError {
    fn into_response(self) -> Response {
        match self {
            Self::Unauthenticated => unauthenticated(),
            Self::Body(error) => error.into_response(),
            Self::List(ListError::Team(team)) => team_failure(&team),
            Self::NotFound { .. } | Self::NoDatabase | Self::Registry(_) | Self::List(_) => {
                let status = if matches!(self, Self::NotFound { .. }) {
                    StatusCode::NOT_FOUND
                } else {
                    StatusCode::INTERNAL_SERVER_ERROR
                };
                let detail = DetailValue::Text(self.to_string());
                (status, Json(DetailBody { detail })).into_response()
            }
        }
    }
}
