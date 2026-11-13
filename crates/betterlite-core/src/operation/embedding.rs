use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use betterlite_openai::chat::deployment::ChatDeployment;
use betterlite_openai::chat::provider_error::{ErrorBodyKind, ProviderFailure, provider_failure};
use betterlite_openai::embedding::{
    EmbeddingError, EmbeddingRequest as ClientEmbeddingRequest, EmbeddingResponse,
};
use betterlite_redact::secret::Redactor;
use http::HeaderMap;
use thiserror::Error;

#[derive(serde::Deserialize)]
pub(crate) struct ProviderErrorBody {
    error: ProviderErrorFields,
}

#[derive(serde::Deserialize)]
pub(crate) struct ProviderErrorFields {
    message: String,
    #[serde(rename = "type")]
    error_type: String,
    param: Option<String>,
    code: Option<String>,
}

fn provider_message(status: u16, body: &[u8], fallback: &EmbeddingError) -> String {
    if let Ok(parsed) = serde_json::from_slice::<ProviderErrorBody>(body) {
        let error = parsed.error;
        return format!(
            "Error code: {status} - {{'error': {{'message': '{}', 'type': '{}', 'param': {}, 'code': {}}}}}",
            error.message,
            error.error_type,
            error
                .param
                .map_or_else(|| String::from("None"), |value| format!("'{value}'")),
            error
                .code
                .map_or_else(|| String::from("None"), |value| format!("'{value}'")),
        );
    }
    fallback.to_string()
}

pub struct EmbeddingRequest {
    pub call_id: String,
    pub model_id: String,
    pub request: Box<ClientEmbeddingRequest>,
    pub calls: Arc<dyn EmbeddingCall>,
}

// the existing embedding implementation sits behind this seam, so the pipeline stays in betterlite-core
#[async_trait]
pub trait EmbeddingCall: Send + Sync {
    async fn embed(
        &self,
        call: EmbeddingCallInput<'_>,
    ) -> Result<EmbeddingResult, EmbeddingFailure>;
}

pub struct EmbeddingCallInput<'call> {
    pub deployment: ChatDeployment,
    pub request: &'call ClientEmbeddingRequest,
}

pub struct EmbeddingResult {
    pub response: EmbeddingResponse,
    pub upstream_headers: HeaderMap,
    pub upstream_time: Duration,
}

#[derive(Debug, Error)]
pub enum EmbeddingFailure {
    #[error(transparent)]
    Route(#[from] EmbeddingError),
    #[error("{message}")]
    Internal { message: String },
    #[error("the provider answered a different operation than the one sent")]
    Mismatched,
}

pub struct EmbeddingRoute {
    pub failure: ProviderFailure,
    pub message: String,
    pub upstream_headers: Option<HeaderMap>,
}

impl EmbeddingRoute {
    #[must_use]
    pub fn internal(message: String) -> Self {
        Self {
            failure: ProviderFailure {
                status: 500,
                error_type: String::from("internal_server_error"),
                param: None,
            },
            message,
            upstream_headers: None,
        }
    }
}

#[must_use]
pub fn route_error(error: &EmbeddingError, redactor: &Redactor) -> EmbeddingRoute {
    match error {
        EmbeddingError::Provider { response } => EmbeddingRoute {
            failure: provider_failure(response, redactor, ErrorBodyKind::MessageOnly),
            message: provider_message(response.status, &response.body, error),
            upstream_headers: Some(response.headers.clone()),
        },
        EmbeddingError::UnknownModel { .. }
        | EmbeddingError::Body { .. }
        | EmbeddingError::Unsupported { .. } => plain_route(400, "invalid_request_error", error),
        EmbeddingError::MissingApiKey => plain_route(401, "authentication_error", error),
        EmbeddingError::UnsupportedProvider { .. } => {
            plain_route(501, "internal_server_error", error)
        }
        EmbeddingError::Http { .. }
        | EmbeddingError::Decode { .. }
        | EmbeddingError::Encode { .. }
        | EmbeddingError::Endpoint { .. }
        | EmbeddingError::Mock { .. } => plain_route(500, "internal_server_error", error),
    }
}

fn plain_route(status: u16, error_type: &str, error: &EmbeddingError) -> EmbeddingRoute {
    EmbeddingRoute {
        failure: ProviderFailure {
            status,
            error_type: error_type.to_owned(),
            param: None,
        },
        message: error.to_string(),
        upstream_headers: None,
    }
}

impl EmbeddingFailure {
    #[must_use]
    pub fn route(&self, redactor: &Redactor) -> EmbeddingRoute {
        match self {
            Self::Route(route) => route_error(route, redactor),
            Self::Internal { message } => {
                EmbeddingRoute::internal(redactor.redact_internal_details(message))
            }
            Self::Mismatched => EmbeddingRoute::internal(String::from(
                "the provider answered a different operation",
            )),
        }
    }
}

mod tests;
