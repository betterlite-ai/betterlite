use std::convert::Infallible;

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use betterlite_auth::auth_user_lookup::error::UserLookupError;
use betterlite_db::postgres_store::errors::{DbError, backend};
use betterlite_db::team_repository::decoder::TeamRecordError;
use betterlite_db::team_repository::repository::TeamQueryError;
use serde::Serialize;

use crate::management_admins::ports::TeamReadError;

#[derive(Debug, thiserror::Error)]
pub enum VisibilityError {
    // _spend_log_forbidden: the same 403 for a foreign row and for no row at all
    #[error("Not authorized to view spend log for request_id={request_id}")]
    Forbidden { request_id: String },
    #[error("the spend log table could not be read")]
    Database(#[source] Box<DbError>),
    #[error("the spend log owner's team could not be read")]
    Team(#[source] Box<TeamQueryError>),
    #[error("the caller's teams could not be read")]
    TeamList(#[source] Box<TeamReadError>),
    #[error("a stored team row cannot be decoded")]
    TeamRecord(#[source] Box<TeamRecordError>),
    #[error("the caller's user row could not be read")]
    Users(#[source] Box<UserLookupError>),
}

impl From<DbError> for VisibilityError {
    fn from(error: DbError) -> Self {
        Self::Database(Box::new(error))
    }
}

impl From<tokio_postgres::Error> for VisibilityError {
    fn from(driver: tokio_postgres::Error) -> Self {
        Self::Database(Box::new(backend::<Infallible>(driver)))
    }
}

#[derive(Serialize)]
struct ForbiddenBody {
    detail: ForbiddenDetail,
}

#[derive(Serialize)]
struct ForbiddenDetail {
    error: String,
}

// HTTPException(403, detail={"error": ...}) renders as {"detail": {"error": ...}}; a read failure is unhandled
impl IntoResponse for VisibilityError {
    fn into_response(self) -> Response {
        match self {
            Self::Forbidden { .. } => {
                let body = ForbiddenBody {
                    detail: ForbiddenDetail {
                        error: self.to_string(),
                    },
                };
                (StatusCode::FORBIDDEN, Json(body)).into_response()
            }
            Self::Database(_)
            | Self::Team(_)
            | Self::TeamList(_)
            | Self::TeamRecord(_)
            | Self::Users(_) => {
                tracing::error!(error = %self, "the spend log visibility check failed");
                (StatusCode::INTERNAL_SERVER_ERROR, "Internal Server Error").into_response()
            }
        }
    }
}
