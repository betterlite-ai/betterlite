use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Serialize;
use thiserror::Error;

use crate::management_guards::error::ManagementGuardError;
use crate::tracing_routes::router::unauthenticated;

use super::body::BodyError;

#[derive(Debug, Error)]
pub(super) enum UsageAiChatError {
    #[error("the API key is not valid")]
    Unauthenticated,
    #[error(transparent)]
    Body(#[from] BodyError),
    #[error(transparent)]
    Caller(#[from] ManagementGuardError),
}

#[derive(Serialize)]
struct DetailBody {
    detail: DetailError,
}

#[derive(Serialize)]
struct DetailError {
    error: String,
}

impl IntoResponse for UsageAiChatError {
    fn into_response(self) -> Response {
        match self {
            Self::Unauthenticated => unauthenticated(),
            Self::Body(error) => error.into_response(),
            Self::Caller(error) => {
                let status = error
                    .status_code()
                    .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
                let body = DetailBody {
                    detail: DetailError {
                        error: error.to_string(),
                    },
                };
                (status, Json(body)).into_response()
            }
        }
    }
}
