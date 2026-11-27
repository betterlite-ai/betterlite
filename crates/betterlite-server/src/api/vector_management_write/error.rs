use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use betterlite_logging::private_json::error::PrivateJsonError;
use betterlite_models::decisions::Json as DecisionJson;
use serde::Serialize;
use thiserror::Error;

use super::cache::CacheError;
use super::params::ParamsError;
use crate::api::vector_management_boundaries::error::VectorManagementError;
use crate::api::vector_management_read::error::ManagementReadError;
use crate::search_tool_routes::body::BodyError;

#[derive(Debug, Error)]
pub enum ManagementWriteError {
    // the principal, the key and the feature switch come from B1070's stage
    #[error(transparent)]
    Read(#[from] ManagementReadError),
    // FastAPI's 422 for a body that does not fit B1131's record
    #[error(transparent)]
    Request(#[from] BodyError),
    // B2486's 400/403/404 and its catch-all failures
    #[error(transparent)]
    Management(#[from] VectorManagementError),
    #[error("Database not connected")]
    DatabaseNotConnected,
    #[error("Vector store with ID {vector_store_id} already exists")]
    AlreadyExists { vector_store_id: String },
    #[error("vector_store_id and custom_llm_provider are required")]
    RequiredFields,
    #[error("Access denied: You do not have permission to delete this vector store")]
    DeleteAccessDenied,
    #[error(transparent)]
    Params(#[from] ParamsError),
    #[error(transparent)]
    PrivateJson(#[from] PrivateJsonError),
    #[error(transparent)]
    Cache(#[from] CacheError),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

#[derive(Serialize)]
struct DetailBody {
    detail: DecisionJson,
}

impl ManagementWriteError {
    // the raises a handler rethrows without logging: HTTPExceptions in LiteLLM, a 400/403/404 here
    #[must_use]
    pub fn is_http_exception(&self) -> bool {
        match self {
            Self::Read(error) => error.is_http_exception(),
            Self::Management(error) => error.http().is_some(),
            Self::DatabaseNotConnected
            | Self::AlreadyExists { .. }
            | Self::RequiredFields
            | Self::DeleteAccessDenied
            | Self::Params(_) => true,
            Self::Request(_) | Self::PrivateJson(_) | Self::Cache(_) | Self::Json(_) => false,
        }
    }
}

fn detail(status: StatusCode, message: String) -> Response {
    (
        status,
        Json(DetailBody {
            detail: DecisionJson::Text(message),
        }),
    )
        .into_response()
}

impl IntoResponse for ManagementWriteError {
    fn into_response(self) -> Response {
        match self {
            Self::Read(error) => error.into_response(),
            Self::Request(error) => error.into_response(),
            // B2486's own rendering keeps the config-owned object and the team failure statuses
            Self::Management(error) => ManagementReadError::from(error).into_response(),
            Self::AlreadyExists { .. } | Self::RequiredFields | Self::Params(_) => {
                detail(StatusCode::BAD_REQUEST, self.to_string())
            }
            Self::DeleteAccessDenied => detail(StatusCode::FORBIDDEN, self.to_string()),
            Self::DatabaseNotConnected | Self::PrivateJson(_) | Self::Cache(_) | Self::Json(_) => {
                detail(StatusCode::INTERNAL_SERVER_ERROR, self.to_string())
            }
        }
    }
}
