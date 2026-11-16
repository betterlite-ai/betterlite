use std::error::Error as StdError;

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use betterlite_prompts::registry::prompt_endpoint_contracts::spec_conversion::PromptRowConversionError;
use serde::Serialize;
use thiserror::Error;

#[derive(Serialize)]
struct DetailBody {
    detail: String,
}

#[derive(Debug, Error)]
pub(super) enum PromptReadError<TableError>
where
    TableError: StdError + 'static,
{
    #[error("{detail}")]
    Denied { status: StatusCode, detail: String },
    #[error("the prompt key's metadata.prompts is not a list of strings")]
    KeyMetadata(#[source] serde_json::Error),
    #[error("a stored prompt row is not a valid prompt")]
    Row(#[source] PromptRowConversionError),
    #[error("the prompt table query failed")]
    Table(#[source] TableError),
}

impl<TableError> IntoResponse for PromptReadError<TableError>
where
    TableError: StdError + 'static,
{
    fn into_response(self) -> Response {
        let status = match &self {
            Self::Denied { status, .. } => *status,
            Self::KeyMetadata(_) => StatusCode::BAD_REQUEST,
            Self::Row(_) | Self::Table(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };
        let body = Json(DetailBody {
            detail: self.to_string(),
        });
        (status, body).into_response()
    }
}
