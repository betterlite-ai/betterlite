use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use betterlite_logging::callback_logger::error::LoggerError;

use super::dates::DetailDateError;
use crate::spend::totals::responses::detail_error;
use crate::spend::visibility::error::VisibilityError;

#[derive(Debug, thiserror::Error)]
pub enum LogPayloadError {
    #[error(
        "Cannot authorize spend log access without a database connection. Connect a database or use a proxy admin key."
    )]
    NoDatabase,
    #[error(transparent)]
    Visibility(#[from] VisibilityError),
    #[error("the caller's identity could not be read")]
    Caller(#[source] serde_json::Error),
    #[error("a spend log detail date could not be read")]
    Dates(#[from] DetailDateError),
    #[error("a logging callback could not read the request payload")]
    Callback(#[source] LoggerError),
}

// the 403s are HTTPExceptions; a bad date or a failing callback is unhandled in LiteLLM, so it is a bare 500
impl IntoResponse for LogPayloadError {
    fn into_response(self) -> Response {
        match self {
            Self::NoDatabase => detail_error(StatusCode::FORBIDDEN, &self.to_string()),
            Self::Visibility(refused) => refused.into_response(),
            Self::Caller(_) | Self::Dates(_) | Self::Callback(_) => {
                tracing::error!(error = %self, "the spend log payload could not be served");
                (StatusCode::INTERNAL_SERVER_ERROR, "Internal Server Error").into_response()
            }
        }
    }
}
