use std::collections::BTreeMap;

use betterlite_http::error::HttpError;
use betterlite_openai::aleph_alpha::error::AlephAlphaError;
use http::header::HeaderMap;
use thiserror::Error;

use crate::endpoint_target::error::EndpointTargetError;
use crate::gemini_stream::error::StreamError;
use crate::model_policy::error::VertexAiError;
use crate::token_cache::error::CredentialCacheError;
use crate::vertex_embedding_call::post::PostError;

// LiteLLM raises this message with status 408 when a complete call times out
const TIMEOUT_MESSAGE: &str = "Timeout error occurred.";

// The literal Vertex returns when a function declaration's OBJECT parameters carry no properties
const EMPTY_PROPERTIES: &str = "GenerateContentRequest.tools[0].function_declarations[0].parameters.properties: should be non-empty for OBJECT type";

#[derive(Debug, Error)]
pub enum GeminiHttpError {
    #[error("the Vertex credentials could not be read: {source}")]
    Credentials {
        #[source]
        source: CredentialCacheError,
    },
    #[error("the Gemini endpoint could not be built: {source}")]
    Target {
        #[source]
        source: EndpointTargetError,
    },
    #[error("the {field} value must be true or false")]
    Flag { field: &'static str },
    #[error("the Gemini request could not be written: {source}")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
    // LiteLLM turns its client's timeout into a VertexAIError with this status and message
    #[error(transparent)]
    Timeout(VertexAiError),
    #[error("the Gemini request could not be sent: {source}")]
    Transport {
        #[source]
        source: PostError,
    },
    #[error(transparent)]
    Provider(VertexAiError),
}

impl GeminiHttpError {
    pub(super) fn transport(source: PostError) -> Self {
        match source {
            PostError::Timeout => Self::Timeout(VertexAiError {
                status_code: 408,
                message: String::from(TIMEOUT_MESSAGE),
                headers: None,
            }),
            other @ (PostError::Url(_)
            | PostError::HeaderName { .. }
            | PostError::HeaderValue { .. }
            | PostError::Send { .. }) => Self::Transport { source: other },
        }
    }

    // LiteLLM hands its VertexAIError to the exception mapper; every other arm is a local failure
    #[must_use]
    pub const fn provider(&self) -> Option<&VertexAiError> {
        match self {
            Self::Provider(error) | Self::Timeout(error) => Some(error),
            Self::Credentials { .. }
            | Self::Target { .. }
            | Self::Flag { .. }
            | Self::Encode { .. }
            | Self::Transport { .. } => None,
        }
    }
}

#[derive(Debug, Error)]
pub enum GeminiStreamError {
    #[error(transparent)]
    Http(#[from] GeminiHttpError),
    #[error("the Gemini response stream timed out")]
    Timeout,
    #[error("the Gemini response line could not be read: {source}")]
    Line {
        #[source]
        source: AlephAlphaError,
    },
    #[error("the Gemini response body could not be read: {source}")]
    Body {
        #[source]
        source: HttpError,
    },
    #[error("the Gemini streaming error body is over the {limit} byte limit")]
    TooLarge { limit: usize },
    #[error(transparent)]
    Chunk(#[from] StreamError),
}

impl GeminiStreamError {
    pub(super) fn body_error(source: HttpError) -> Self {
        let timed_out = match &source {
            HttpError::Timeout => true,
            HttpError::Request(error) => error.is_timeout(),
            HttpError::Required { .. }
            | HttpError::DotSegment { .. }
            | HttpError::UnsupportedProxyScheme { .. }
            | HttpError::InvalidProxyUrl { .. }
            | HttpError::Client(_)
            | HttpError::TooLarge { .. } => false,
        };
        if timed_out {
            return Self::Timeout;
        }
        Self::Body { source }
    }
}

// LiteLLM's translate_exception_str: one Vertex complaint about an empty OBJECT schema becomes a caller-facing message
#[must_use]
pub fn translate_exception_str(message: String) -> String {
    if message.contains(EMPTY_PROPERTIES) {
        return format!(
            "'properties' field in tools[0]['function']['parameters'] cannot be empty if 'type' == 'object'. Received error from provider - {message}"
        );
    }
    message
}

// LiteLLM's get_error_class: the response status and headers ride the provider error unchanged
#[must_use]
pub fn get_error_class(message: String, status_code: u16, headers: &HeaderMap) -> VertexAiError {
    VertexAiError {
        status_code,
        message,
        headers: Some(error_headers(headers)),
    }
}

fn error_headers(headers: &HeaderMap) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    for (name, value) in headers {
        let text: String = value
            .as_bytes()
            .iter()
            .map(|byte| char::from(*byte))
            .collect();
        map.entry(name.as_str().to_owned())
            .and_modify(|joined: &mut String| {
                joined.push_str(", ");
                joined.push_str(&text);
            })
            .or_insert(text);
    }
    map
}
