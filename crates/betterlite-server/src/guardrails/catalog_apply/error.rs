use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use betterlite_error::envelope::{ErrorObject, ErrorResponse, WireField};
use betterlite_guardrails::guardrail_base::failure::GuardrailFailure;
use betterlite_guardrails::guardrail_records::error::GuardrailRecordError;
use serde::Serialize;
use thiserror::Error;

use crate::guardrails::callback_caps::errors::ProxyHookError;

use super::ports::ApplyLogError;

const INVALID_CATEGORY_NAME: &str = "Invalid category name";
const INTERNAL_TYPE: &str = "internal_server_error";

#[derive(Debug, Error)]
pub enum CatalogApplyError {
    #[error("Invalid category name")]
    InvalidCategoryName,
    #[error("Category file not found: {category_name} (tried .yaml and .json)")]
    CategoryNotFound { category_name: String },
    #[error("Error reading category file: {source}")]
    CategoryRead {
        #[source]
        source: std::io::Error,
    },
    #[error("Error reading category file: {source}")]
    CategoryBlocking {
        #[source]
        source: tokio::task::JoinError,
    },
    #[error("Error reading major_airlines.json: {source}")]
    AirlinesRead {
        #[source]
        source: serde_json::Error,
    },
    #[error(
        "Guardrail '{guardrail_name}' not found. Please ensure the guardrail is configured in your LiteLLM proxy."
    )]
    GuardrailNotFound { guardrail_name: String },
    #[error(transparent)]
    Guardrail(#[from] GuardrailFailure),
    #[error(transparent)]
    Hooks(#[from] ProxyHookError),
    #[error(transparent)]
    Log(#[from] ApplyLogError),
    #[error("the guardrail invocation record could not be written")]
    Record(#[from] GuardrailRecordError),
}

impl CatalogApplyError {
    #[must_use]
    pub fn status(&self) -> StatusCode {
        let status = match self {
            Self::InvalidCategoryName => 400,
            Self::CategoryNotFound { .. } | Self::GuardrailNotFound { .. } => 404,
            Self::CategoryRead { .. }
            | Self::CategoryBlocking { .. }
            | Self::AirlinesRead { .. }
            | Self::Log(_)
            | Self::Record(_) => 500,
            Self::Guardrail(failure) => failure_status(failure),
            Self::Hooks(error) => hook_status(error),
        };
        StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR)
    }

    #[must_use]
    pub fn detail(&self) -> String {
        match self {
            Self::InvalidCategoryName => INVALID_CATEGORY_NAME.to_owned(),
            Self::CategoryNotFound { .. }
            | Self::CategoryRead { .. }
            | Self::CategoryBlocking { .. }
            | Self::AirlinesRead { .. }
            | Self::GuardrailNotFound { .. }
            | Self::Guardrail(_)
            | Self::Hooks(_)
            | Self::Log(_)
            | Self::Record(_) => self.to_string(),
        }
    }

    // LiteLLM skips the logging handlers only for an HTTPException, and a ProxyException is not one
    pub(super) const fn is_http(&self) -> bool {
        matches!(
            self,
            Self::GuardrailNotFound { .. }
                | Self::Guardrail(
                    GuardrailFailure::Http { .. } | GuardrailFailure::HttpObject { .. }
                )
                | Self::Hooks(ProxyHookError::Http { .. })
        )
    }
}

impl IntoResponse for CatalogApplyError {
    fn into_response(self) -> Response {
        match &self {
            Self::InvalidCategoryName => {
                detail_response(StatusCode::BAD_REQUEST, INVALID_CATEGORY_NAME)
            }
            Self::CategoryNotFound { .. }
            | Self::CategoryRead { .. }
            | Self::CategoryBlocking { .. }
            | Self::AirlinesRead { .. } => detail_response(self.status(), &self.detail()),
            Self::GuardrailNotFound { .. } => {
                let status = self.status().as_u16();
                proxy_error(
                    status,
                    &self.detail(),
                    INTERNAL_TYPE,
                    None,
                    &status.to_string(),
                )
            }
            Self::Guardrail(failure) => failure_response(failure),
            Self::Hooks(error) => hook_response(error),
            Self::Log(_) | Self::Record(_) => {
                tracing::error!(error = ?self, "apply_guardrail: the guardrail run could not be logged");
                proxy_error(500, &self.detail(), INTERNAL_TYPE, None, "500")
            }
        }
    }
}

const fn failure_status(failure: &GuardrailFailure) -> u16 {
    match failure {
        GuardrailFailure::GuardrailRaised { status_code, .. }
        | GuardrailFailure::BlockedPii { status_code, .. } => *status_code,
        GuardrailFailure::Http { status, .. } | GuardrailFailure::HttpObject { status, .. } => {
            *status
        }
        GuardrailFailure::ModifyResponse(_)
        | GuardrailFailure::SensitiveDataRoute(_)
        | GuardrailFailure::Other(_) => 500,
    }
}

fn hook_status(error: &ProxyHookError) -> u16 {
    match error {
        ProxyHookError::Http { status, .. } | ProxyHookError::GuardrailRaised { status, .. } => {
            *status
        }
        ProxyHookError::Proxy(exception) => proxy_code_status(&exception.code),
        ProxyHookError::SensitiveDataRoute(_)
        | ProxyHookError::ModifyResponse(_)
        | ProxyHookError::RejectedRequest { .. }
        | ProxyHookError::Other(_) => 500,
    }
}

fn proxy_code_status(code: &str) -> u16 {
    code.trim()
        .parse::<u16>()
        .ok()
        .filter(|status| (100..=999).contains(status))
        .unwrap_or(500)
}

fn failure_response(failure: &GuardrailFailure) -> Response {
    let status = failure_status(failure);
    let message = match failure {
        GuardrailFailure::Http { detail, .. } => detail.clone(),
        _ => failure.to_string(),
    };
    proxy_error(status, &message, INTERNAL_TYPE, None, &status.to_string())
}

fn hook_response(error: &ProxyHookError) -> Response {
    let status = hook_status(error);
    let message = match error {
        ProxyHookError::Proxy(exception) => {
            return proxy_error(
                proxy_code_status(&exception.code),
                &exception.message,
                &exception.error_type,
                exception.param.as_deref(),
                &exception.code,
            );
        }
        ProxyHookError::Http { detail, .. } => detail.clone(),
        ProxyHookError::GuardrailRaised { message, .. } => message.clone(),
        ProxyHookError::SensitiveDataRoute(_)
        | ProxyHookError::ModifyResponse(_)
        | ProxyHookError::RejectedRequest { .. }
        | ProxyHookError::Other(_) => error.to_string(),
    };
    proxy_error(status, &message, INTERNAL_TYPE, None, &status.to_string())
}

fn proxy_error(
    status: u16,
    message: &str,
    error_type: &str,
    param: Option<&str>,
    code: &str,
) -> Response {
    let body = ErrorResponse {
        error: ErrorObject {
            message: message.to_owned(),
            error_type: WireField::Text(error_type.to_owned()),
            param: param
                .filter(|text| *text != "None")
                .map_or(WireField::Null, |text| WireField::Text(text.to_owned())),
            code: WireField::Text(code.to_owned()),
        },
    };
    let status = StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    (status, Json(body)).into_response()
}

#[derive(Serialize)]
struct DetailBody {
    detail: String,
}

fn detail_response(status: StatusCode, detail: &str) -> Response {
    (
        status,
        Json(DetailBody {
            detail: detail.to_owned(),
        }),
    )
        .into_response()
}
