use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse as _, Response};
use betterlite_error::envelope::{ErrorObject, ErrorResponse, WireField};
use betterlite_error::proxy_error_types::common::CommonProxyErrors;
use serde::Serialize;

use crate::config_overlay_port::ConfigOverlayPortError;
use crate::proxy_config_state::error::{ConfigOwnedDetail, ConfigStateError};

const STORE_MODEL_IN_DB_MESSAGE: &str =
    "Set `'STORE_MODEL_IN_DB='True'` in your env to enable this feature.";
const DELETE_FAILURE_PREFIX: &str = "Error deleting callback: ";
const GET_FAILURE_PREFIX: &str = "Authentication Error, ";

// The checks answer a {"error": <text>} detail, and the catch-all blocks answer the ProxyException envelope with str(code)
#[derive(Debug, thiserror::Error)]
pub enum CallbackConfigError {
    #[error("{}", CommonProxyErrors::DbNotConnectedError.as_str())]
    DatabaseNotConnected,
    #[error("{}", CommonProxyErrors::NotAllowedAccess.as_str())]
    NotAllowedAccess { role: String },
    #[error("Set `'STORE_MODEL_IN_DB='True'` in your env to enable this feature.")]
    StorageDisabled,
    #[error("Callback '{name}' not found in active configuration")]
    CallbackNotFound { name: String },
    #[error("{field} must be {expected}")]
    InvalidConfig {
        field: String,
        expected: &'static str,
    },
    #[error("{0}")]
    Overlay(#[from] ConfigOverlayPortError),
    #[error("{0}")]
    State(#[from] ConfigStateError),
    #[error("{0}")]
    Json(#[from] serde_json::Error),
}

impl CallbackConfigError {
    pub(crate) fn into_delete_response(self) -> Response {
        match self {
            Self::DatabaseNotConnected => detail_error(
                StatusCode::BAD_REQUEST,
                CommonProxyErrors::DbNotConnectedError.as_str(),
            ),
            Self::NotAllowedAccess { role } => detail_error(
                StatusCode::BAD_REQUEST,
                &format!(
                    "{}, your role={role}",
                    CommonProxyErrors::NotAllowedAccess.as_str()
                ),
            ),
            Self::StorageDisabled => {
                detail_error(StatusCode::INTERNAL_SERVER_ERROR, STORE_MODEL_IN_DB_MESSAGE)
            }
            Self::CallbackNotFound { name } => detail_error(
                StatusCode::NOT_FOUND,
                &format!("Callback '{name}' not found in active configuration"),
            ),
            Self::State(ConfigStateError::ConfigOwned(detail)) => {
                (StatusCode::BAD_REQUEST, Json(OwnedDetail { detail })).into_response()
            }
            other @ (Self::InvalidConfig { .. }
            | Self::Overlay(_)
            | Self::State(_)
            | Self::Json(_)) => proxy_failure(
                StatusCode::INTERNAL_SERVER_ERROR,
                &format!("{DELETE_FAILURE_PREFIX}{other}"),
                "internal_server_error",
                "callback_name",
                "500",
            ),
        }
    }

    pub(crate) fn into_get_response(self) -> Response {
        proxy_failure(
            StatusCode::BAD_REQUEST,
            &format!("{GET_FAILURE_PREFIX}{self}"),
            "auth_error",
            "None",
            "400",
        )
    }
}

#[derive(Serialize)]
struct DetailText<'text> {
    detail: DetailError<'text>,
}

#[derive(Serialize)]
struct DetailError<'text> {
    error: &'text str,
}

#[derive(Serialize)]
struct OwnedDetail {
    detail: Box<ConfigOwnedDetail>,
}

fn detail_error(status: StatusCode, message: &str) -> Response {
    (
        status,
        Json(DetailText {
            detail: DetailError { error: message },
        }),
    )
        .into_response()
}

fn proxy_failure(
    status: StatusCode,
    message: &str,
    error_type: &str,
    param: &str,
    code: &str,
) -> Response {
    let body = ErrorResponse {
        error: ErrorObject {
            message: message.to_owned(),
            error_type: WireField::Text(error_type.to_owned()),
            param: WireField::Text(param.to_owned()),
            code: WireField::Text(code.to_owned()),
        },
    };
    (status, Json(body)).into_response()
}
