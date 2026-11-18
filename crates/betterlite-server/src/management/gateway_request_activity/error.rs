use axum::Json;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::response::Response;
use betterlite_db::postgres_store::errors::DbError;
use betterlite_error::proxy_error_types::common::CommonProxyErrors;
use serde::Serialize;
use thiserror::Error;

use crate::tracing_routes::router::unauthenticated;

#[derive(Debug, Error)]
pub(super) enum ActivityError {
    #[error("the request has no authenticated caller")]
    Unauthenticated,
    #[error("{0}")]
    Forbidden(&'static str),
    #[error("the database is not connected")]
    DbNotConnected,
    #[error("the gateway activity query failed")]
    Store(#[source] DbError),
}

#[derive(Debug, Serialize)]
struct DetailBody {
    detail: &'static str,
}

#[derive(Debug, Serialize)]
struct InternalBody {
    error: InternalMessage,
}

#[derive(Debug, Serialize)]
struct InternalMessage {
    message: &'static str,
    #[serde(rename = "type")]
    error_type: &'static str,
}

impl IntoResponse for ActivityError {
    fn into_response(self) -> Response {
        match self {
            Self::Unauthenticated => unauthenticated(),
            Self::Forbidden(detail) => {
                (StatusCode::FORBIDDEN, Json(DetailBody { detail })).into_response()
            }
            Self::DbNotConnected => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(DetailBody {
                    detail: CommonProxyErrors::DbNotConnectedError.as_str(),
                }),
            )
                .into_response(),
            Self::Store(error) => {
                tracing::warn!(error = %error, "the gateway activity query failed");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(InternalBody {
                        error: InternalMessage {
                            message: "Internal server error",
                            error_type: "internal_server_error",
                        },
                    }),
                )
                    .into_response()
            }
        }
    }
}
