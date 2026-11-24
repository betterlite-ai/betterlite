use std::fmt;

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use betterlite_auth::request_body_safety::SafetyError;
use betterlite_error::proxy_error_types::common::CommonProxyErrors;
use serde::Serialize;

use crate::auth::auth_resource_grants::denial_response;
use crate::health_access_and_connections::connection_state::ConnectionProbeError;
use crate::health_access_and_connections::credential_scope::EnvironmentReference;
use crate::health_access_and_connections::visibility::CallerError;
use crate::health_probe_settings::model_settings::HealthSettingsError;
use crate::model_write_fields::error::ModelWriteError;
use crate::tracing_routes::router::unauthenticated;
use crate::workflow_runs::error::ValidationFailure;

const VIEW_ONLY_DETAIL: &str = "View-only users cannot manage models.";
const TEAM_ID_DETAIL: &str = "the model_info team_id must be a string";

// LiteLLM raises HTTPException with a text detail or with {"error": ...}; both land under {"detail": ...}
#[derive(Debug, thiserror::Error)]
#[error("{detail}")]
pub struct Rejection {
    pub status: StatusCode,
    pub detail: RejectionDetail,
}

#[derive(Debug, thiserror::Error)]
pub enum RejectionDetail {
    #[error("{0}")]
    Message(String),
    #[error("{0}")]
    Error(String),
}

#[derive(Debug, thiserror::Error)]
pub enum ConnectionError {
    #[error("the request has no authenticated caller")]
    Unauthenticated,
    #[error(transparent)]
    Validation(#[from] ValidationFailure),
    #[error("the database is not connected")]
    Database,
    #[error(transparent)]
    Environment(#[from] EnvironmentReference),
    #[error(transparent)]
    RequestBody(#[from] SafetyError),
    #[error(transparent)]
    Caller(#[from] CallerError),
    #[error(transparent)]
    Preparation(#[from] HealthSettingsError),
    #[error("{0}")]
    Rejection(Rejection),
    #[error(transparent)]
    Probe(#[from] ConnectionProbeError),
    #[error("the model_info team_id must be a string")]
    TeamIdType,
    #[error("the random generator is unavailable: {0}")]
    Random(String),
    #[error("{message}")]
    Internal { message: String },
    #[error(transparent)]
    Encode(#[from] serde_json::Error),
}

// can_user_make_model_call's three refusals, in the exact texts the model management endpoint raises
#[must_use]
pub fn model_call_rejection(error: &ModelWriteError, role: Option<&str>) -> ConnectionError {
    let (status, detail) = match error {
        ModelWriteError::ViewOnly => (
            StatusCode::FORBIDDEN,
            RejectionDetail::Message(VIEW_ONLY_DETAIL.to_owned()),
        ),
        ModelWriteError::TeamMissing { team_id } => (
            StatusCode::BAD_REQUEST,
            RejectionDetail::Error(format!("Team id={team_id} does not exist in db")),
        ),
        ModelWriteError::NotTeamAdmin { team_id, key_team } => (
            StatusCode::FORBIDDEN,
            RejectionDetail::Error(format!(
                "Team ID={team_id} does not match the API key's team ID={}, OR you are not the admin for this team. Check `/user/info` to verify your team admin status.",
                key_team.as_deref().unwrap_or("None")
            )),
        ),
        ModelWriteError::NotProxyAdmin => (
            StatusCode::FORBIDDEN,
            RejectionDetail::Error(format!(
                "User does not have permission to make this model call. Your role={}. You can only make model calls if you are a PROXY_ADMIN or if you are a team admin, by specifying a team_id in the model_info.",
                role.unwrap_or("None")
            )),
        ),
        ModelWriteError::OrphanedModel
        | ModelWriteError::MemberRouterUnavailable
        | ModelWriteError::AdminOnly { .. }
        | ModelWriteError::TeamRead(_)
        | ModelWriteError::TeamAccess(_) => {
            return ConnectionError::Internal {
                message: error.to_string(),
            };
        }
    };
    ConnectionError::Rejection(Rejection { status, detail })
}

#[derive(Serialize)]
struct TextDetailBody {
    detail: String,
}

#[derive(Serialize)]
struct ErrorDetailBody {
    detail: ErrorField,
}

#[derive(Serialize)]
struct ErrorField {
    error: String,
}

impl IntoResponse for Rejection {
    fn into_response(self) -> Response {
        match self.detail {
            RejectionDetail::Message(message) => {
                (self.status, Json(TextDetailBody { detail: message })).into_response()
            }
            RejectionDetail::Error(message) => error_detail(self.status, message),
        }
    }
}

impl IntoResponse for ConnectionError {
    fn into_response(self) -> Response {
        match self {
            Self::Unauthenticated => unauthenticated(),
            Self::Validation(failure) => failure.into_response(),
            Self::Database => error_detail(
                StatusCode::INTERNAL_SERVER_ERROR,
                CommonProxyErrors::DbNotConnectedError.as_str().to_owned(),
            ),
            Self::Environment(environment) => {
                error_detail(StatusCode::BAD_REQUEST, environment.to_string())
            }
            Self::RequestBody(error) => denial_response(&error.into_proxy_exception()),
            Self::Rejection(rejection) => rejection.into_response(),
            Self::Caller(error) => failed_to_test(&error),
            Self::Preparation(error) => failed_to_test(&error),
            Self::Probe(error) => failed_to_test(&error),
            Self::Encode(error) => failed_to_test(&error),
            Self::TeamIdType => failed_to_test(&TEAM_ID_DETAIL),
            Self::Random(message) | Self::Internal { message } => failed_to_test(&message),
        }
    }
}

fn error_detail(status: StatusCode, message: String) -> Response {
    (
        status,
        Json(ErrorDetailBody {
            detail: ErrorField { error: message },
        }),
    )
        .into_response()
}

// LiteLLM's except block: HTTPException passes through, every other exception becomes this 500
fn failed_to_test<E>(error: &E) -> Response
where
    E: fmt::Display,
{
    error_detail(
        StatusCode::INTERNAL_SERVER_ERROR,
        format!("Failed to test connection: {error}"),
    )
}
