use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use crate::health_access_and_connections::connection_error::{Rejection, RejectionDetail};
use crate::health_access_and_connections::visibility::{CallerError, LookupFailure};
use crate::tracing_routes::router::unauthenticated;

// Every failure the three routes raise: an HTTPException's status and detail dict, or a re-raised internal error
#[derive(Debug, thiserror::Error)]
pub enum HealthRouteError {
    #[error("the request has no authenticated caller")]
    Unauthenticated,
    #[error(transparent)]
    Caller(#[from] CallerError),
    #[error(transparent)]
    Lookup(#[from] LookupFailure),
    #[error("Database not initialized")]
    Database,
    #[error("Model list not initialized")]
    ModelListNotInitialized,
    #[error(transparent)]
    Http(Rejection),
    #[error("{0}")]
    Internal(String),
}

impl HealthRouteError {
    // HTTPException(status_code=status, detail={"error": message}), rendered under {"detail": ...}
    #[must_use]
    pub fn http(status: StatusCode, message: String) -> Self {
        Self::Http(Rejection {
            status,
            detail: RejectionDetail::Error(message),
        })
    }
}

impl IntoResponse for HealthRouteError {
    fn into_response(self) -> Response {
        match self {
            Self::Unauthenticated => unauthenticated(),
            Self::Caller(error) => internal_text(error.to_string()),
            Self::Lookup(error) => internal_text(error.to_string()),
            Self::Database => internal_error("Database not initialized"),
            Self::ModelListNotInitialized => internal_error("Model list not initialized"),
            Self::Http(rejection) => rejection.into_response(),
            Self::Internal(message) => internal_text(message),
        }
    }
}

fn internal_error(message: &'static str) -> Response {
    Rejection {
        status: StatusCode::INTERNAL_SERVER_ERROR,
        detail: RejectionDetail::Error(message.to_owned()),
    }
    .into_response()
}

fn internal_text(message: String) -> Response {
    Rejection {
        status: StatusCode::INTERNAL_SERVER_ERROR,
        detail: RejectionDetail::Message(message),
    }
    .into_response()
}
