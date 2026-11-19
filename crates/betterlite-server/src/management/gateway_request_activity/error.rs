use axum::Json;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::response::Response;
use betterlite_db::database_faults::native::is_native_transport_error;
use betterlite_db::postgres_store::errors::DbError;
use betterlite_error::envelope::ErrorObject;
use betterlite_error::envelope::ErrorResponse;
use betterlite_error::envelope::WireField;
use betterlite_error::proxy_error_types::common::CommonProxyErrors;
use serde::Serialize;
use thiserror::Error;
use tokio_postgres::error::DbError as DriverDbError;

use crate::tracing_routes::router::unauthenticated;

pub(super) const DB_UNAVAILABLE: &str = "the gateway activity database is unavailable";
const CACHED_PLAN_MESSAGE: &str = "cached plan must not change result type";

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
            Self::Store(error) if is_database_outage(&error) => {
                tracing::warn!(error = %error, "the gateway activity database is unavailable");
                database_unavailable()
            }
            Self::Store(error) => {
                tracing::warn!(error = %error, "the gateway activity query failed");
                internal_error()
            }
        }
    }
}

fn is_database_outage(error: &DbError) -> bool {
    if is_native_transport_error(error) {
        return true;
    }
    driver_message(error)
        .is_some_and(|message| message.to_ascii_lowercase().contains(CACHED_PLAN_MESSAGE))
}

fn driver_message(error: &DbError) -> Option<&str> {
    let source = match error {
        DbError::Backend { source }
        | DbError::SqlState { source, .. }
        | DbError::UniqueViolation { source, .. } => source,
        DbError::NoDbConnected
        | DbError::Pool { .. }
        | DbError::InvalidRecord { .. }
        | DbError::CreateNull => return None,
    };
    source.driver.as_db_error().map(DriverDbError::message)
}

fn database_unavailable() -> Response {
    let body = ErrorResponse {
        error: ErrorObject {
            message: DB_UNAVAILABLE.to_owned(),
            error_type: WireField::Text("no_db_connection".to_owned()),
            param: WireField::MISSING,
            code: WireField::Text("503".to_owned()),
        },
    };
    (StatusCode::SERVICE_UNAVAILABLE, Json(body)).into_response()
}

fn internal_error() -> Response {
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
