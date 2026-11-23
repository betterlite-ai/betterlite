use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Serialize;
use thiserror::Error;

use crate::tracing_routes::router::unauthenticated;

use super::hours::{HoursFailure, ValidationBody, ValidationIssue};
use super::ports::{ReloadStoreError, StoreFuture};

pub(super) const ADMIN_ROLE_REQUIRED: &str = "Access denied. Admin role required. Current role:";
pub(super) const HOURS_NOT_POSITIVE: &str = "Hours must be greater than 0";
pub(super) const DATABASE_NOT_AVAILABLE: &str = "Database connection not available";

#[derive(Debug, Error)]
pub(super) enum ReloadError {
    #[error("the request has no authenticated caller")]
    Unauthenticated,
    #[error("the caller's role {role} may not call this route")]
    Forbidden { role: String },
    #[error(transparent)]
    HoursQuery(#[from] HoursFailure),
    #[error("the requested interval is not positive")]
    HoursNotPositive,
    #[error("the database is not connected")]
    MissingDatabase,
    #[error("the model cost map source is unavailable: {reason}")]
    CostMapUnavailable { reason: String },
    #[error("{prefix}: {cause}")]
    Wrapped {
        prefix: &'static str,
        #[source]
        cause: WrappedCause,
    },
}

// str() of the HTTPException the wrapped handlers catch, or the store failure they re-raise
#[derive(Debug, Error)]
pub(super) enum WrappedCause {
    #[error("500: Database connection not available")]
    MissingDatabase,
    #[error("the interval {hours} hours does not fit the stored schedule")]
    Interval { hours: i64 },
    #[error(transparent)]
    Store(#[from] ReloadStoreError),
}

impl ReloadError {
    // the 500 a handler's catch block wraps a missing database in, "500: Database connection not available"
    pub(super) fn missing_database(prefix: &'static str) -> Self {
        Self::Wrapped {
            prefix,
            cause: WrappedCause::MissingDatabase,
        }
    }

    pub(super) fn wrapped(prefix: &'static str, source: ReloadStoreError) -> Self {
        Self::Wrapped {
            prefix,
            cause: WrappedCause::Store(source),
        }
    }
}

pub(super) async fn store_call<Value>(
    call: StoreFuture<'_, Value>,
    prefix: &'static str,
) -> Result<Value, ReloadError> {
    call.await
        .map_err(|source| ReloadError::wrapped(prefix, source))
}

#[derive(Serialize)]
struct DetailBody {
    detail: String,
}

// the status and the {"detail": text} body every non-validation HTTPException serializes to
struct DetailResponse(StatusCode, String);

impl IntoResponse for DetailResponse {
    fn into_response(self) -> Response {
        (self.0, Json(DetailBody { detail: self.1 })).into_response()
    }
}

// FastAPI's 422 body, {"detail": [{"type", "loc", "msg"}]}
struct ValidationResponse(ValidationIssue);

impl IntoResponse for ValidationResponse {
    fn into_response(self) -> Response {
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(ValidationBody { detail: [self.0] }),
        )
            .into_response()
    }
}

impl IntoResponse for ReloadError {
    fn into_response(self) -> Response {
        match self {
            Self::Unauthenticated => unauthenticated(),
            Self::Forbidden { role } => DetailResponse(
                StatusCode::FORBIDDEN,
                format!("{ADMIN_ROLE_REQUIRED} {role}"),
            )
            .into_response(),
            Self::HoursQuery(failure) => ValidationResponse(failure.issue()).into_response(),
            Self::HoursNotPositive => {
                DetailResponse(StatusCode::BAD_REQUEST, HOURS_NOT_POSITIVE.to_owned())
                    .into_response()
            }
            Self::MissingDatabase => DetailResponse(
                StatusCode::INTERNAL_SERVER_ERROR,
                DATABASE_NOT_AVAILABLE.to_owned(),
            )
            .into_response(),
            Self::CostMapUnavailable { reason } => DetailResponse(
                StatusCode::BAD_GATEWAY,
                format!(
                    "Failed to reload model cost map: {reason}. Current pricing data was kept."
                ),
            )
            .into_response(),
            wrapped @ Self::Wrapped { .. } => {
                DetailResponse(StatusCode::INTERNAL_SERVER_ERROR, wrapped.to_string())
                    .into_response()
            }
        }
    }
}
