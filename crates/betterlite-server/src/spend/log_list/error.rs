use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use betterlite_error::envelope::{ErrorObject, ErrorResponse, WireField};

use super::store::ListStoreError;
use crate::spend::log_query::prepare::PrepareError;
use crate::spend::log_query::validate::{ChoiceError, QueryIssues};
use crate::spend::log_results::session::SessionAggregateError;
use crate::spend::sessions::ports::SessionsStoreError;
use crate::spend::visibility::error::VisibilityError;

const NO_DATABASE_MESSAGE: &str = "Prisma Client is not initialized";
const NONE_SENTINEL: &str = "None";
const INTERNAL_ERROR_TYPE: &str = "internal_error";
const CONVERTED_ERROR_TYPE: &str = "internal_server_error";
const GROUPED_UNAVAILABLE: &str = "the grouped spend log read is unavailable";

#[derive(Debug, thiserror::Error)]
pub enum LogListError {
    // the converted envelope sends the caught failure's own message, so Display stays the source's
    #[error("{0}")]
    Identity(#[source] serde_json::Error),
    // FastAPI rejects an undecodable query parameter before the endpoint runs
    #[error("the spend log query does not match the declared types")]
    Validation(QueryIssues),
    #[error("the database is not connected")]
    NoDatabase,
    #[error(transparent)]
    Choices(#[from] ChoiceError),
    #[error(transparent)]
    Prepare(#[from] PrepareError),
    #[error("Not authorized to view spend log for request_id={request_id}")]
    ForbiddenRequestId { request_id: String },
    #[error("Not authorized to view team spend for team_id={team_id}")]
    ForbiddenTeam { team_id: String },
    #[error(transparent)]
    Visibility(VisibilityError),
    #[error(transparent)]
    Store(#[from] ListStoreError),
    #[error(transparent)]
    Sessions(#[from] SessionsStoreError),
    #[error(transparent)]
    Aggregate(#[from] SessionAggregateError),
    #[error("the grouped spend log page has no database to read")]
    GroupedStore,
}

// _assert_user_can_view_request_id raises the same 403 the detail endpoint answers
impl From<VisibilityError> for LogListError {
    fn from(error: VisibilityError) -> Self {
        match error {
            VisibilityError::Forbidden { request_id } => Self::ForbiddenRequestId { request_id },
            other @ (VisibilityError::Database(_)
            | VisibilityError::Team(_)
            | VisibilityError::TeamList(_)
            | VisibilityError::TeamRecord(_)
            | VisibilityError::Users(_)) => Self::Visibility(other),
        }
    }
}

// a ProxyException raised before the try block keeps its own type and param
fn proxy_error(
    message: String,
    error_type: &str,
    param: WireField,
    status: StatusCode,
) -> Response {
    let body = ErrorResponse {
        error: ErrorObject {
            message,
            error_type: WireField::Text(error_type.to_owned()),
            param,
            code: WireField::Text(status.as_u16().to_string()),
        },
    };
    (status, Json(body)).into_response()
}

// handle_exception_on_proxy above the handler's except block: a caught failure keeps its status and sends internal_server_error with a null param, while a ProxyException raised before the try keeps its own type and param
impl IntoResponse for LogListError {
    fn into_response(self) -> Response {
        match self {
            Self::Validation(issues) => issues.into_response(),
            Self::Choices(error) => error.into_response(),
            Self::Prepare(error) => error.into_response(),
            Self::NoDatabase => proxy_error(
                NO_DATABASE_MESSAGE.to_owned(),
                INTERNAL_ERROR_TYPE,
                WireField::Text(NONE_SENTINEL.to_owned()),
                StatusCode::UNAUTHORIZED,
            ),
            Self::ForbiddenRequestId { .. } | Self::ForbiddenTeam { .. } => proxy_error(
                self.to_string(),
                CONVERTED_ERROR_TYPE,
                WireField::Null,
                StatusCode::FORBIDDEN,
            ),
            Self::Identity(_)
            | Self::Visibility(_)
            | Self::Store(_)
            | Self::Sessions(_)
            | Self::Aggregate(_) => proxy_error(
                self.to_string(),
                CONVERTED_ERROR_TYPE,
                WireField::Null,
                StatusCode::INTERNAL_SERVER_ERROR,
            ),
            Self::GroupedStore => proxy_error(
                GROUPED_UNAVAILABLE.to_owned(),
                CONVERTED_ERROR_TYPE,
                WireField::Null,
                StatusCode::INTERNAL_SERVER_ERROR,
            ),
        }
    }
}
