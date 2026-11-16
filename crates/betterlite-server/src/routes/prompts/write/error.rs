use std::error::Error as StdError;
use std::iter::once;

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use betterlite_prompts::registry::mutation::prompt_registry_mutation::error::PromptRegistryError;
use betterlite_prompts::registry::prompt_endpoint_contracts::records::AMBIGUOUS_PROMPT_DATA_ERROR;
use betterlite_prompts::registry::prompt_endpoint_contracts::spec_conversion::PromptRowConversionError;
use betterlite_prompts::registry::prompt_endpoint_contracts::table::PromptVersionError;
use serde::Serialize;
use thiserror::Error;

use super::wire::RequestBodyError;

#[derive(Debug, Error)]
pub(super) enum PromptWriteError<TableError>
where
    TableError: StdError + 'static,
{
    #[error("{0}")]
    Body(#[from] RequestBodyError),
    #[error("Only proxy admins can {action} prompts")]
    Forbidden { action: &'static str },
    #[error(
        "DB not connected. This endpoint needs a database; set DATABASE_URL to a PostgreSQL connection string (postgresql://...) to enable it. See https://docs.litellm.ai/docs/proxy/virtual_keys"
    )]
    DbNotConnected,
    #[error("{}", AMBIGUOUS_PROMPT_DATA_ERROR)]
    Ambiguous,
    #[error("Cannot {action} config prompts.")]
    ConfigPrompt { action: &'static str },
    #[error("{detail}")]
    NotFound { detail: String },
    #[error("{source}")]
    Table {
        #[source]
        source: TableError,
    },
    #[error("{source}")]
    Version {
        #[source]
        source: PromptVersionError<TableError>,
    },
    #[error("{0}")]
    Stored(#[from] PromptRowConversionError),
    #[error("{0}")]
    Registry(#[from] PromptRegistryError),
    #[error("{0}")]
    Encode(#[from] serde_json::Error),
}

#[derive(Serialize)]
struct DetailBody {
    detail: String,
}

#[derive(Serialize)]
struct ValidationBody {
    detail: Vec<ValidationIssue>,
}

#[derive(Serialize)]
struct ValidationIssue {
    #[serde(rename = "type")]
    issue_type: &'static str,
    loc: Vec<String>,
    msg: String,
}

impl<TableError> PromptWriteError<TableError>
where
    TableError: StdError + 'static,
{
    const fn status(&self) -> StatusCode {
        match self {
            Self::Body(_) => StatusCode::UNPROCESSABLE_ENTITY,
            Self::Forbidden { .. } => StatusCode::FORBIDDEN,
            Self::Ambiguous | Self::ConfigPrompt { .. } => StatusCode::BAD_REQUEST,
            Self::NotFound { .. } => StatusCode::NOT_FOUND,
            Self::DbNotConnected
            | Self::Table { .. }
            | Self::Version { .. }
            | Self::Stored(_)
            | Self::Registry(_)
            | Self::Encode(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

// FastAPI answers a body that does not fit the model with a 422 and a list of located issues
impl<TableError> IntoResponse for PromptWriteError<TableError>
where
    TableError: StdError + 'static,
{
    fn into_response(self) -> Response {
        let status = self.status();
        if let Self::Body(body_error) = &self {
            let location = once("body".to_owned())
                .chain(
                    body_error
                        .path
                        .split('.')
                        .filter(|part| !part.is_empty())
                        .map(str::to_owned),
                )
                .collect();
            let issue = ValidationIssue {
                issue_type: "value_error",
                loc: location,
                msg: body_error.source.to_string(),
            };
            let body = ValidationBody {
                detail: vec![issue],
            };
            return (status, Json(body)).into_response();
        }
        let body = DetailBody {
            detail: self.to_string(),
        };
        (status, Json(body)).into_response()
    }
}
