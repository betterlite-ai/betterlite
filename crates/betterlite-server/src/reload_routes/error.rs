use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Serialize;
use thiserror::Error;

use crate::tracing_routes::router::unauthenticated;

use super::hours::{HoursFailure, ValidationBody};
use super::ports::ReloadStoreError;

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
    pub(super) fn wrapped(prefix: &'static str, cause: WrappedCause) -> Self {
        Self::Wrapped { prefix, cause }
    }
}

#[derive(Serialize)]
struct DetailBody {
    detail: String,
}

impl IntoResponse for ReloadError {
    fn into_response(self) -> Response {
        match self {
            Self::Unauthenticated => unauthenticated(),
            Self::Forbidden { role } => detail(
                StatusCode::FORBIDDEN,
                format!("{ADMIN_ROLE_REQUIRED} {role}"),
            ),
            Self::HoursQuery(failure) => validation(&failure),
            Self::HoursNotPositive => {
                detail(StatusCode::BAD_REQUEST, HOURS_NOT_POSITIVE.to_owned())
            }
            Self::MissingDatabase => detail(
                StatusCode::INTERNAL_SERVER_ERROR,
                DATABASE_NOT_AVAILABLE.to_owned(),
            ),
            Self::CostMapUnavailable { reason } => detail(
                StatusCode::BAD_GATEWAY,
                format!(
                    "Failed to reload model cost map: {reason}. Current pricing data was kept."
                ),
            ),
            wrapped @ Self::Wrapped { .. } => {
                detail(StatusCode::INTERNAL_SERVER_ERROR, wrapped.to_string())
            }
        }
    }
}

fn detail(status: StatusCode, text: String) -> Response {
    (status, Json(DetailBody { detail: text })).into_response()
}

fn validation(failure: &HoursFailure) -> Response {
    let body = ValidationBody {
        detail: [failure.issue()],
    };
    (StatusCode::UNPROCESSABLE_ENTITY, Json(body)).into_response()
}
