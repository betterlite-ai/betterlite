use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Serialize;

use super::ports::SessionsStoreError;
use super::query::SessionQueryIssues;

const NO_DATABASE_DETAIL: &str = "Database not connected";

#[derive(Debug, thiserror::Error)]
pub enum SessionsError {
    #[error("the caller's identity could not be read")]
    Identity(#[source] serde_json::Error),
    // FastAPI rejects an unparsable or missing query parameter before the endpoint runs
    #[error("the session query does not match the declared types")]
    Validation(SessionQueryIssues),
    #[error("Database not connected")]
    NoDatabase,
    // any other failure becomes HTTPException(500, detail=str(e))
    #[error(transparent)]
    Store(#[from] SessionsStoreError),
}

// HTTPException(status_code=500, detail="...") renders as {"detail": "..."}
#[derive(Serialize)]
struct DetailBody {
    detail: String,
}

impl IntoResponse for SessionsError {
    fn into_response(self) -> Response {
        let (status, detail) = match self {
            Self::Validation(issues) => {
                return (StatusCode::UNPROCESSABLE_ENTITY, Json(issues)).into_response();
            }
            Self::Identity(source) => (StatusCode::INTERNAL_SERVER_ERROR, source.to_string()),
            Self::NoDatabase => (
                StatusCode::INTERNAL_SERVER_ERROR,
                String::from(NO_DATABASE_DETAIL),
            ),
            Self::Store(error) => {
                tracing::error!(error = %error, "the session spend log read failed");
                (StatusCode::INTERNAL_SERVER_ERROR, error.to_string())
            }
        };
        (status, Json(DetailBody { detail })).into_response()
    }
}
