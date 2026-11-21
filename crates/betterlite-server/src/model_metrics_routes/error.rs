use axum::Json;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::response::Response;
use betterlite_db::postgres_store::errors::DbError;
use betterlite_error::envelope::ErrorObject;
use betterlite_error::envelope::ErrorResponse;
use betterlite_error::envelope::WireField;
use betterlite_error::proxy_error_types::common::CommonProxyErrors;
use serde::Serialize;
use thiserror::Error;

use crate::error_body::json_error;

const PRISMA_NOT_INITIALIZED: &str = "Prisma Client is not initialized";

#[derive(Debug, Error)]
pub(super) enum ModelMetricsError {
    #[error("the streaming metrics database is not connected")]
    StreamingDatabaseMissing,
    #[error("the model metrics database is not initialized")]
    DatabaseMissing,
    #[error("{parameter} is not an ISO 8601 datetime")]
    InvalidDatetime {
        parameter: &'static str,
        value: String,
    },
    #[error("the alerting threshold is not a whole number of seconds")]
    AlertingThreshold,
    #[error("the model metrics query failed")]
    Store(#[source] DbError),
}

impl IntoResponse for ModelMetricsError {
    fn into_response(self) -> Response {
        match self {
            Self::StreamingDatabaseMissing => {
                proxy_internal_error(CommonProxyErrors::DbNotConnectedError.as_str())
            }
            Self::DatabaseMissing => proxy_internal_error(PRISMA_NOT_INITIALIZED),
            Self::InvalidDatetime { parameter, value } => invalid_datetime(parameter, &value),
            Self::AlertingThreshold => {
                tracing::error!("the alerting threshold cannot be cut to whole seconds");
                generic_internal_error()
            }
            Self::Store(source) => {
                tracing::error!(error = %source, "the model metrics query failed");
                generic_internal_error()
            }
        }
    }
}

#[derive(Serialize)]
struct DetailMessage {
    error: String,
}

#[derive(Serialize)]
struct DetailBody {
    detail: DetailMessage,
}

fn invalid_datetime(parameter: &str, value: &str) -> Response {
    let message = format!("{parameter} must be an ISO 8601 datetime, got {value:?}");
    (
        StatusCode::BAD_REQUEST,
        Json(DetailBody {
            detail: DetailMessage { error: message },
        }),
    )
        .into_response()
}

fn proxy_internal_error(message: &str) -> Response {
    let body = ErrorResponse {
        error: ErrorObject {
            message: message.to_owned(),
            error_type: WireField::Text("internal_error".to_owned()),
            param: WireField::Text("None".to_owned()),
            code: WireField::Text("500".to_owned()),
        },
    };
    (StatusCode::INTERNAL_SERVER_ERROR, Json(body)).into_response()
}

fn generic_internal_error() -> Response {
    json_error(
        StatusCode::INTERNAL_SERVER_ERROR,
        "Internal server error",
        "internal_server_error",
        "500",
    )
}
