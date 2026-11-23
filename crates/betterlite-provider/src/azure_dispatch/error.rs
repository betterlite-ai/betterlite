use crate::operation::OperationError;
use betterlite_azure::azure_urls::AzureUrlError;
use betterlite_azure::chat_call::error::AzureChatError;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_models::params::ParameterError;
use betterlite_prompts::arguments::json::{Json, Object};
use http::HeaderMap;
use http::header::{InvalidHeaderName, InvalidHeaderValue};
use serde_json::Error as JsonError;
use std::io::Error as IoError;
use std::time::TryFromFloatSecsError;
use thiserror::Error;
use url::ParseError as UrlParseError;
#[derive(Debug, Error)]
pub enum AzureProviderError {
    #[error(
        "api_base is required for Azure OpenAI LLM provider. Either set it dynamically or set the AZURE_API_BASE environment variable."
    )]
    MissingTextApiBase,
    #[error("No API Base provided for Azure OpenAI LLM provider. Set 'AZURE_API_BASE' in .env")]
    MissingEmbeddingApiBase,
    #[error("extra_body must be an object")]
    ExtraBodyNotAnObject,
    #[error("extra_body.azure_ad_token must be a string or null")]
    ExtraBodyToken,
    #[error("max retries must be an int")]
    MaxRetriesType,
    #[error("{message}")]
    Rejected { status: u16, message: String },
    #[error("extra_headers must be an object")]
    HeadersNotObject,
    #[error("the {name} header value must be a string")]
    HeaderValueType { name: String },
    #[error("the {name} header holds a value HTTP cannot carry: {source}")]
    Header {
        name: String,
        #[source]
        source: InvalidHeaderValue,
    },
    #[error(transparent)]
    HeaderName(#[from] InvalidHeaderName),
    #[error("the Azure URL could not be built: {source}")]
    Url {
        #[source]
        source: AzureUrlError,
    },
    #[error(transparent)]
    InvalidUrl(#[from] UrlParseError),
    #[error("the request could not be written as JSON: {source}")]
    Encode {
        #[source]
        source: JsonError,
    },
    #[error(transparent)]
    TimeoutValue(#[from] TryFromFloatSecsError),
    #[error(transparent)]
    Transport(#[from] TransportError),
}
// the status a Python exception carries: a raised ValueError has none
impl AzureProviderError {
    #[must_use]
    pub const fn status(&self) -> Option<u16> {
        match self {
            Self::Rejected { status, .. } => Some(*status),
            Self::MaxRetriesType => Some(422),
            Self::InvalidUrl(_) | Self::HeaderName(_) => Some(400),
            Self::TimeoutValue(_) => Some(500),
            Self::MissingTextApiBase
            | Self::MissingEmbeddingApiBase
            | Self::ExtraBodyNotAnObject
            | Self::ExtraBodyToken
            | Self::HeadersNotObject
            | Self::HeaderValueType { .. }
            | Self::Header { .. }
            | Self::Url { .. }
            | Self::Encode { .. }
            | Self::Transport(_) => None,
        }
    }
}
impl From<AzureChatError> for AzureProviderError {
    fn from(error: AzureChatError) -> Self {
        let status = match &error {
            AzureChatError::Azure { status, .. } => *status,
            AzureChatError::Cancelled => 500,
        };
        Self::Rejected {
            status,
            message: error.text(),
        }
    }
}
fn provider_error(status: u16, kind: &str, message: String) -> OperationError {
    let error = Object::from_iter([
        (String::from("message"), Json::Text(message)),
        (String::from("type"), Json::Text(kind.to_owned())),
    ]);
    let envelope = Object::from_iter([(String::from("error"), Json::Object(error))]);
    let body = serde_json::to_vec(&Json::Object(envelope)).unwrap_or_default();
    OperationError::Provider {
        response: Box::new(UpstreamResponse {
            status,
            headers: HeaderMap::new(),
            body,
        }),
    }
}
fn parameters(message: &str) -> OperationError {
    OperationError::Parameters {
        source: ParameterError::Encode {
            source: JsonError::io(IoError::other(message.to_owned())),
        },
    }
}
impl From<AzureProviderError> for OperationError {
    fn from(error: AzureProviderError) -> Self {
        match error {
            failure @ (AzureProviderError::MissingTextApiBase
            | AzureProviderError::MissingEmbeddingApiBase) => {
                provider_error(500, "ValueError", failure.to_string())
            }
            failure @ (AzureProviderError::ExtraBodyNotAnObject
            | AzureProviderError::ExtraBodyToken) => parameters(&failure.to_string()),
            failure @ (AzureProviderError::HeadersNotObject
            | AzureProviderError::HeaderValueType { .. }
            | AzureProviderError::HeaderName(_)
            | AzureProviderError::InvalidUrl(_)) => {
                provider_error(400, "AzureOpenAIError", failure.to_string())
            }
            failure @ AzureProviderError::MaxRetriesType => {
                provider_error(422, "AzureOpenAIError", failure.to_string())
            }
            AzureProviderError::Rejected { status, message } => {
                provider_error(status, "AzureOpenAIError", message)
            }
            AzureProviderError::Header { name, source } => Self::InvalidHeader { name, source },
            AzureProviderError::TimeoutValue(source) => {
                provider_error(500, "AzureOpenAIError", source.to_string())
            }
            AzureProviderError::Url { source } => parameters(&source.to_string()),
            AzureProviderError::Encode { source } => Self::Encode { source },
            AzureProviderError::Transport(source) => Self::from(source),
        }
    }
}
