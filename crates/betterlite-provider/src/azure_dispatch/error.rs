use std::io::Error as IoError;

use betterlite_azure::azure_urls::AzureUrlError;
use betterlite_azure::chat_call::error::AzureChatError;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_models::params::ParameterError;
use betterlite_prompts::arguments::json::{Json, Object};
use http::HeaderMap;
use http::header::InvalidHeaderValue;
use serde_json::Error as JsonError;
use thiserror::Error;

use crate::operation::OperationError;

pub const MISSING_TEXT_BASE: &str = "api_base is required for Azure OpenAI LLM provider. Either set it dynamically or set the AZURE_API_BASE environment variable.";
pub const MISSING_EMBEDDING_BASE: &str =
    "No API Base provided for Azure OpenAI LLM provider. Set 'AZURE_API_BASE' in .env";
pub const MAX_RETRIES_TYPE: &str = "max retries must be an int";

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
    #[error("the Azure URL could not be built: {source}")]
    Url {
        #[source]
        source: AzureUrlError,
    },
    #[error("the request could not be written as JSON: {source}")]
    Encode {
        #[source]
        source: JsonError,
    },
    #[error("the provider answer is not the expected {operation} response: {source}")]
    Decode {
        operation: &'static str,
        #[source]
        source: JsonError,
    },
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
            Self::MissingTextApiBase
            | Self::MissingEmbeddingApiBase
            | Self::ExtraBodyNotAnObject
            | Self::ExtraBodyToken
            | Self::HeadersNotObject
            | Self::HeaderValueType { .. }
            | Self::Header { .. }
            | Self::Url { .. }
            | Self::Encode { .. }
            | Self::Decode { .. }
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

// A status and an error body, the way the route renders a provider's own answer.
fn provider_error(status: u16, kind: &str, message: String) -> OperationError {
    let error = Object::from_iter([
        (String::from("message"), Json::Text(message)),
        (String::from("type"), Json::Text(kind.to_owned())),
    ]);
    let body = Object::from_iter([(String::from("error"), Json::Object(error))]);
    let body = serde_json::to_vec(&Json::Object(body)).unwrap_or_default();
    OperationError::Provider {
        response: Box::new(UpstreamResponse {
            status,
            headers: HeaderMap::new(),
            body,
        }),
    }
}

impl From<AzureProviderError> for OperationError {
    fn from(source: AzureProviderError) -> Self {
        match source {
            failure @ (AzureProviderError::MissingTextApiBase
            | AzureProviderError::MissingEmbeddingApiBase) => {
                provider_error(500, "ValueError", failure.to_string())
            }
            AzureProviderError::ExtraBodyNotAnObject => OperationError::Parameters {
                source: ParameterError::Encode {
                    source: JsonError::io(IoError::other("extra_body must be an object")),
                },
            },
            AzureProviderError::ExtraBodyToken => OperationError::Parameters {
                source: ParameterError::Encode {
                    source: JsonError::io(IoError::other(
                        "extra_body.azure_ad_token must be a string or null",
                    )),
                },
            },
            AzureProviderError::MaxRetriesType => {
                provider_error(422, "AzureOpenAIError", String::from(MAX_RETRIES_TYPE))
            }
            AzureProviderError::Rejected { status, message } => {
                provider_error(status, "AzureOpenAIError", message)
            }
            AzureProviderError::HeadersNotObject => provider_error(
                400,
                "AzureOpenAIError",
                String::from("extra_headers must be an object"),
            ),
            AzureProviderError::HeaderValueType { name } => provider_error(
                400,
                "AzureOpenAIError",
                format!("the {name} header value must be a string"),
            ),
            AzureProviderError::Header { name, source } => {
                OperationError::InvalidHeader { name, source }
            }
            AzureProviderError::Url { source } => OperationError::Parameters {
                source: ParameterError::Encode {
                    source: JsonError::io(IoError::other(source.to_string())),
                },
            },
            AzureProviderError::Encode { source } => OperationError::Encode { source },
            AzureProviderError::Decode { operation, source } => {
                OperationError::Decode { operation, source }
            }
            AzureProviderError::Transport(source) => OperationError::from(source),
        }
    }
}
