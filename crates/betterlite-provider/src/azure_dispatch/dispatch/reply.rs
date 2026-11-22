use std::collections::BTreeMap;
use std::io::Error as IoError;

use betterlite_azure::chat_call::response::project as chat_project;
use betterlite_azure::completion::response::project as text_project;
use betterlite_azure::embeddings::client::{Parsed, RawEmbedding};
use betterlite_azure::embeddings::error::EmbeddingError;
use betterlite_azure::embeddings::reply::response_error;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_models::decisions::Json as Body;
use betterlite_openai::chat::response::ChatResponse;
use betterlite_openai::compatible_wire::{ReplyOptions, system_clock};
use betterlite_openai::embedding::EmbeddingResponse;
use betterlite_openai::text_completion_transport::logging::SecretHeaders;
use betterlite_openai::text_completion_transport::reply::CompletionReply;
use betterlite_prompts::arguments::json::{Json, Object};
use serde_json::Error as JsonError;
use uuid::Uuid;

use crate::operation::{OperationError, success};

use super::super::error::AzureProviderError;

pub(super) fn decode_error(operation: &'static str, source: JsonError) -> OperationError {
    OperationError::Decode { operation, source }
}

// convert_to_model_response_object over the answer body, with the id and created LiteLLM invents
pub(super) fn chat_response(
    response: UpstreamResponse,
    json_mode: bool,
) -> Result<ChatResponse, OperationError> {
    let checked = success(response)?;
    let body: Body =
        serde_json::from_slice(&checked.body).map_err(|source| decode_error("chat", source))?;
    let headers = SecretHeaders::from_response(&checked.headers);
    let options = ReplyOptions {
        json_mode,
        provider: None,
        base_model: None,
        identifier: Uuid::new_v4,
        clock: system_clock,
    };
    chat_project(&body, &headers, &options, false)
        .map(|reply| reply.response)
        .map_err(|error| decode_error("chat", JsonError::io(IoError::other(error.text()))))
}

pub(super) fn embedding_response(
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
    let parsed = raw.parse().map_err(|error| {
        decode_error(
            "embedding",
            JsonError::io(IoError::other(error.to_string())),
        )
    })?;
    match parsed {
        Parsed::Embedding(response) => {
            if let Some(EmbeddingError::Response {
                status_code,
                message,
            }) = response_error(&raw.body)
            {
                return Err(AzureProviderError::Rejected {
                    status: status_code,
                    message,
                }
                .into());
            }
            Ok(*response)
        }
        Parsed::Text(text) => Err(decode_error(
            "embedding",
            JsonError::io(IoError::other(format!(
                "Unexpected string response from Azure: {text}"
            ))),
        )),
    }
}

// The text completion the Azure SDK parses, converted to the chat-shaped reply LiteLLM returns
pub(super) fn text_response(response: UpstreamResponse) -> Result<ChatResponse, OperationError> {
    let checked = success(response)?;
    let parsed: CompletionReply =
        serde_json::from_slice(&checked.body).map_err(|source| decode_error("text", source))?;
    let projected = text_project(&parsed, &|| Uuid::new_v4(), &system_clock, false)
        .map_err(|error| decode_error("text", JsonError::io(IoError::other(error.to_string()))))?;
    let value =
        serde_json::to_value(&projected).map_err(|source| OperationError::Encode { source })?;
    serde_json::from_value(value).map_err(|source| decode_error("text", source))
}

pub(super) fn chat_reply_view(response: &ChatResponse) -> Json {
    let first = response.choices.first();
    let content = first.and_then(|choice| choice.message.content.clone());
    let finish_reason = first.and_then(|choice| choice.finish_reason.clone());
    Json::Object(Object::from_iter([
        (String::from("id"), Json::Text(response.id.clone())),
        (String::from("object"), Json::Text(response.object.clone())),
        (String::from("model"), Json::Text(response.model.clone())),
        (
            String::from("content"),
            content.map_or(Json::Null, Json::Text),
        ),
        (
            String::from("finish_reason"),
            finish_reason.map_or(Json::Null, Json::Text),
        ),
        (
            String::from("total_tokens"),
            Json::Number(response.usage.total_tokens.into()),
        ),
    ]))
}

pub(super) fn embedding_reply_view(response: &EmbeddingResponse) -> Json {
    let embedding = response.data.first().map_or(Json::Null, |record| {
        serde_json::to_string(&record.embedding)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or(Json::Null)
    });
    Json::Object(Object::from_iter([
        (String::from("object"), Json::Text(response.object.clone())),
        (String::from("model"), Json::Text(response.model.clone())),
        (String::from("embedding"), embedding),
        (
            String::from("total_tokens"),
            Json::Number(response.usage.total_tokens.into()),
        ),
    ]))
}
