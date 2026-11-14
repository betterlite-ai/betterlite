use betterlite_http::proxy::UpstreamResponse;
use betterlite_redact::secret::Redactor;
use http::{HeaderMap, HeaderName, HeaderValue};

use crate::chat::provider_error::{ErrorBodyKind, provider_failure};
use crate::text_completion_request::CompletionRequestError;
use crate::text_completion_transport::error::{
    ErrorStatus, TextCompletionError, TextCompletionFailure,
};
use crate::text_completion_transport::logging::SecretHeaders;

use super::catalog::CompletionRoute;

// The status, type, param and message the route answers with; the message is betterlite's own
#[derive(Debug)]
pub struct CompletionFailure {
    pub status: u16,
    pub error_type: String,
    pub param: Option<String>,
    pub message: String,
    pub upstream_headers: HeaderMap,
}

impl CompletionFailure {
    pub(super) fn internal(message: String) -> Self {
        Self {
            status: 500,
            error_type: String::from("internal_server_error"),
            param: None,
            message,
            upstream_headers: HeaderMap::new(),
        }
    }

    pub(super) fn route(route: &CompletionRoute) -> Self {
        match route {
            CompletionRoute::Text { .. } => Self::internal(String::from(
                "betterlite does not serve this completion deployment yet",
            )),
            CompletionRoute::Chat { .. } => Self {
                status: 500,
                error_type: String::from("internal_server_error"),
                param: None,
                message: String::from("betterlite does not serve chat-model text completions yet"),
                upstream_headers: HeaderMap::new(),
            },
            CompletionRoute::Unsupported { provider } => Self {
                status: 501,
                error_type: String::from("internal_server_error"),
                param: None,
                message: format!("betterlite does not serve text completions for {provider}"),
                upstream_headers: HeaderMap::new(),
            },
        }
    }
}

pub(super) fn request_failure(error: CompletionRequestError) -> CompletionFailure {
    CompletionFailure {
        status: 400,
        error_type: String::from("invalid_request_error"),
        param: None,
        message: error.to_string(),
        upstream_headers: HeaderMap::new(),
    }
}

// LiteLLM's OpenAIError carries only the status and headers, so the proxy's type and param follow the status
pub(super) fn transport_failure(
    failure: TextCompletionFailure,
    redactor: &Redactor,
) -> CompletionFailure {
    match failure {
        TextCompletionFailure::Wrapped(error) => wrapped_failure(&error, redactor),
        TextCompletionFailure::Raw(error) => CompletionFailure {
            status: 500,
            error_type: String::from("internal_server_error"),
            param: None,
            message: error.to_string(),
            upstream_headers: HeaderMap::new(),
        },
    }
}

fn response_headers(headers: &SecretHeaders) -> HeaderMap {
    let mut map = HeaderMap::new();
    for (name, value) in &headers.0 {
        let (Ok(name), Ok(value)) = (
            HeaderName::try_from(name.as_str()),
            HeaderValue::try_from(value.as_str()),
        ) else {
            continue;
        };
        map.insert(name, value);
    }
    map
}

fn wrapped_failure(error: &TextCompletionError, redactor: &Redactor) -> CompletionFailure {
    let status = match error.status {
        ErrorStatus::Missing | ErrorStatus::Null => 500,
        ErrorStatus::Code(code) => code,
    };
    let upstream_headers = error
        .headers
        .as_ref()
        .map(response_headers)
        .unwrap_or_default();
    let failure = provider_failure(
        &UpstreamResponse {
            status,
            headers: upstream_headers.clone(),
            body: Vec::new(),
        },
        redactor,
        ErrorBodyKind::MessageOnly,
    );
    CompletionFailure {
        status: failure.status,
        error_type: failure.error_type,
        param: failure.param,
        message: error.message.clone(),
        upstream_headers,
    }
}
