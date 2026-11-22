use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use betterlite_core::api::claude_plugin_policy::{ClaudePluginPolicyError, PluginErrorDetail};
use betterlite_core::api::claude_plugin_write::ClaudePluginWriteFailure;
use serde::Serialize;
use thiserror::Error;

use crate::auth::failures::body_too_large_response;
use crate::error_body::{AuthenticationFailure, authentication_error};
use crate::search_tool_routes::body::BodyError;

// LiteLLM raises HTTPException for every handler failure, so FastAPI renders the detail object
#[derive(Debug, Error)]
pub(crate) enum ClaudePluginWriteError {
    #[error("the request has no authenticated caller")]
    Unauthenticated,
    #[error(transparent)]
    Policy(#[from] ClaudePluginPolicyError),
    #[error("the write input carries no plugin specification")]
    MissingSpec,
    #[error(transparent)]
    Body(#[from] BodyError),
    #[error("the request body is too large")]
    TooLarge { observed: usize, limit: usize },
    #[error("{message}")]
    Backend { message: String },
}

impl From<ClaudePluginWriteFailure> for ClaudePluginWriteError {
    fn from(failure: ClaudePluginWriteFailure) -> Self {
        match failure {
            ClaudePluginWriteFailure::Policy(error) => Self::Policy(error),
            ClaudePluginWriteFailure::MissingSpec => Self::MissingSpec,
        }
    }
}

impl IntoResponse for ClaudePluginWriteError {
    fn into_response(self) -> Response {
        match self {
            Self::Unauthenticated => authentication_error(&AuthenticationFailure::Missing),
            Self::Policy(error) => policy_response(error),
            missing @ Self::MissingSpec => {
                detail_response(StatusCode::INTERNAL_SERVER_ERROR, missing.to_string())
            }
            Self::Body(error) => error.into_response(),
            Self::TooLarge { observed, limit } => body_too_large_response(observed, limit),
            Self::Backend { message } => {
                detail_response(StatusCode::INTERNAL_SERVER_ERROR, message)
            }
        }
    }
}

#[derive(Serialize)]
struct DetailBody {
    detail: PluginErrorDetail,
}

fn policy_response(error: ClaudePluginPolicyError) -> Response {
    let fields = error.fields();
    let status =
        StatusCode::from_u16(fields.status_code).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let body = DetailBody {
        detail: fields.detail,
    };
    (status, Json(body)).into_response()
}

fn detail_response(status: StatusCode, message: String) -> Response {
    let body = DetailBody {
        detail: PluginErrorDetail { error: message },
    };
    (status, Json(body)).into_response()
}
