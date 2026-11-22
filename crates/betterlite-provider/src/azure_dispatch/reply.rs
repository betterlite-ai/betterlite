use super::Azure;
use super::error::AzureProviderError;
use super::resolve::convert;
use crate::operation::{ChatCompletion, OperationError, OperationResult, success};
use crate::prepared::{PreparedKind, PreparedRequest, Upstream};
use crate::provider::Adapter;
use betterlite_azure::chat_call::response::project as chat_project;
use betterlite_azure::completion::response::project as text_project;
use betterlite_azure::embeddings::client::{Parsed, RawEmbedding};
use betterlite_azure::embeddings::error::EmbeddingError;
use betterlite_azure::embeddings::reply::response_error;
use betterlite_context::chat_config_selection::azure::is_o_series_model;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_models::decisions::Json as Body;
use betterlite_openai::chat::response::ChatResponse;
use betterlite_openai::compatible_wire::{ReplyOptions, system_clock};
use betterlite_openai::embedding::EmbeddingResponse;
use betterlite_openai::text_completion_transport::logging::SecretHeaders;
use betterlite_openai::text_completion_transport::reply::CompletionReply;
use serde_json::Error as JsonError;
use std::collections::BTreeMap;
use std::io::Error as IoError;
use uuid::Uuid;
pub(crate) enum Projection<'name> {
    Chat {
        json_mode: bool,
        provider: Option<&'name str>,
    },
    Text,
}
// OpenAIChatCompletion stamps an o-series reply with the provider name, so its model reads azure/<model>.
pub(crate) fn reply_provider(detection: Option<&str>) -> Option<&'static str> {
    match detection {
        Some(model) if is_o_series_model(model) => Some(Azure::NAME),
        Some(_) | None => None,
    }
}
pub(crate) fn decode(
    prepared: &PreparedRequest,
    upstream: Upstream,
) -> Result<OperationResult, OperationError> {
    match (prepared.kind(), upstream) {
        (PreparedKind::Chat(chat), Upstream::Buffered(response)) => Ok(OperationResult::Chat(
            ChatCompletion::OpenAi(Box::new(project(
                response,
                Projection::Chat {
                    json_mode: false,
                    provider: reply_provider(chat.client_model.as_deref()),
                },
            )?)),
        )),
        (PreparedKind::Stream(_), Upstream::Streaming(response)) => {
            Ok(OperationResult::Stream(Box::new(response)))
        }
        (PreparedKind::Stream(_), Upstream::Buffered(response)) => {
            success(response)?;
            Err(unsupported())
        }
        (PreparedKind::Embedding(_), Upstream::Buffered(response)) => Ok(
            OperationResult::Embedding(Box::new(embedding_response(response)?)),
        ),
        (
            PreparedKind::Chat(_) | PreparedKind::Embedding(_) | PreparedKind::Other,
            Upstream::Streaming(_) | Upstream::Anthropic(_),
        )
        | (PreparedKind::Stream(_), Upstream::Anthropic(_))
        | (PreparedKind::Other, Upstream::Buffered(_)) => Err(unsupported()),
    }
}
pub(super) fn unsupported() -> OperationError {
    OperationError::UnsupportedOperation {
        provider: String::from(Azure::NAME),
        operation: String::from("provider endpoint"),
    }
}
pub(crate) fn project(
    response: UpstreamResponse,
    projection: Projection<'_>,
) -> Result<ChatResponse, OperationError> {
    let checked = success(response)?;
    match projection {
        Projection::Chat {
            json_mode,
            provider,
        } => chat_reply(&checked, json_mode, provider),
        Projection::Text => text_reply(&checked),
    }
}
fn chat_reply(
    response: &UpstreamResponse,
    json_mode: bool,
    provider: Option<&str>,
) -> Result<ChatResponse, OperationError> {
    let body: Body =
        serde_json::from_slice(&response.body).map_err(|source| OperationError::Decode {
            operation: "chat",
            source,
        })?;
    let options = ReplyOptions {
        json_mode,
        provider,
        base_model: None,
        identifier: Uuid::new_v4,
        clock: system_clock,
    };
    chat_project(
        &body,
        &SecretHeaders::from_response(&response.headers),
        &options,
        false,
    )
    .map(|reply| reply.response)
    .map_err(|error| OperationError::Decode {
        operation: "chat",
        source: JsonError::io(IoError::other(error.text())),
    })
}
fn text_reply(response: &UpstreamResponse) -> Result<ChatResponse, OperationError> {
    let parsed: CompletionReply =
        serde_json::from_slice(&response.body).map_err(|source| OperationError::Decode {
            operation: "text",
            source,
        })?;
    let projected =
        text_project(&parsed, &Uuid::new_v4, &system_clock, false).map_err(|error| {
            OperationError::Decode {
                operation: "text",
                source: JsonError::io(IoError::other(error.to_string())),
            }
        })?;
    convert(&projected).map_err(OperationError::from)
}
pub(crate) fn embedding_response(
    response: UpstreamResponse,
) -> Result<EmbeddingResponse, OperationError> {
    let checked = success(response)?;
    let headers: BTreeMap<String, String> = checked
        .headers
        .iter()
        .map(|(name, value)| {
            (
                name.as_str().to_owned(),
                value.to_str().unwrap_or_default().to_owned(),
            )
        })
        .collect();
    let raw = RawEmbedding {
        status: checked.status,
        headers,
        body: checked.body,
    };
    let parsed = raw.parse().map_err(|error| OperationError::Decode {
        operation: "embedding",
        source: JsonError::io(IoError::other(error.to_string())),
    })?;
    match parsed {
        Parsed::Embedding(response) => match response_error(&raw.body) {
            Some(EmbeddingError::Response {
                status_code,
                message,
            }) => Err(OperationError::from(AzureProviderError::Rejected {
                status: status_code,
                message,
            })),
            None => Ok(*response),
        },
        Parsed::Text(text) => Err(OperationError::Decode {
            operation: "embedding",
            source: JsonError::io(IoError::other(format!(
                "Unexpected string response from Azure: {text}"
            ))),
        }),
    }
}
