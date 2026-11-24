use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use betterlite_core::api::claude_plugin_policy::{
    ClaudePluginPolicyError, PluginErrorDetail, PluginErrorFields,
};
use betterlite_core::api::claude_plugin_read::error::ClaudePluginReadFailure;
use betterlite_db::auxiliary_repository::repository::TableError;
use serde::Serialize;
use thiserror::Error;

use super::ClaudePluginReadDecodeError;

// every handler failure becomes FastAPI's detail object, and the source stays in the type
#[derive(Debug, Error)]
pub(crate) enum ClaudePluginReadError {
    #[error(transparent)]
    Policy(#[from] ClaudePluginPolicyError),
    #[error(transparent)]
    Read(#[from] ClaudePluginReadFailure),
    #[error(transparent)]
    Decode(#[from] ClaudePluginReadDecodeError),
    #[error(transparent)]
    Table(#[from] TableError),
    #[error("the caller's plugin permissions are not readable")]
    KeyView {
        #[source]
        source: serde_json::Error,
    },
}

impl IntoResponse for ClaudePluginReadError {
    fn into_response(self) -> Response {
        match &self {
            Self::Policy(error) => fields_response(error.fields()),
            Self::Read(failure) => fields_response(failure.fields()),
            Self::KeyView { source } => {
                let message = format!("the caller's plugin permissions are not readable: {source}");
                detail_response(message)
            }
            Self::Decode(_) | Self::Table(_) => detail_response(self.to_string()),
        }
    }
}

#[derive(Serialize)]
struct DetailBody {
    detail: PluginErrorDetail,
}

fn fields_response(fields: PluginErrorFields) -> Response {
    let status =
        StatusCode::from_u16(fields.status_code).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let body = DetailBody {
        detail: fields.detail,
    };
    (status, Json(body)).into_response()
}

// the handlers' 500 branches, with detail {"error": <message>} as LiteLLM writes it
fn detail_response(message: String) -> Response {
    let body = DetailBody {
        detail: PluginErrorDetail { error: message },
    };
    (StatusCode::INTERNAL_SERVER_ERROR, Json(body)).into_response()
}
