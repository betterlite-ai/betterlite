// The answer conversions on both bridges: LiteLLM's convert_to_chat_model_response_object for a
// text answer on the chat route, and convert_chat_to_text_completion for a chat answer on the
// completion route.

use serde_json::{Map, Value};

use super::failure;
use crate::chat::response::ChatResponse;
use crate::text_completion_execution::failure::CompletionFailure;
use crate::text_completion_response::{CompletionChoice, CompletionResponse, CompletionUsage};
use crate::text_completion_stream::without_nulls;

// The fine-tuned branch answers the client with the text body itself, model renamed by the route.
pub fn text_completion_body(
    response: &CompletionResponse,
    model: Option<&str>,
) -> Result<Vec<u8>, CompletionFailure> {
    let mut value = serde_json::to_value(response).map_err(encode_failure)?;
    if let Some(model) = model
        && let Value::Object(fields) = &mut value
    {
        fields.insert(String::from("model"), Value::from(model));
    }
    serde_json::to_vec(&without_nulls(value)).map_err(encode_failure)
}

// convert_to_chat_model_response_object: a fresh chat response carrying the text answer, stamped
// with the call's own time rather than the provider's.
pub fn chat_completion_body(
    response: &CompletionResponse,
    created: i64,
    model: Option<&str>,
) -> Result<Vec<u8>, CompletionFailure> {
    let choices = response
        .choices
        .iter()
        .enumerate()
        .map(|(index, choice)| {
            let mut message = Map::new();
            message.insert(
                String::from("content"),
                choice.text.as_deref().map_or(Value::Null, Value::from),
            );
            message.insert(String::from("role"), Value::from("assistant"));
            let mut converted = Map::new();
            converted.insert(
                String::from("finish_reason"),
                choice
                    .finish_reason
                    .as_deref()
                    .map_or(Value::Null, Value::from),
            );
            converted.insert(String::from("index"), Value::from(index));
            converted.insert(String::from("message"), Value::Object(message));
            if let Some(logprobs) = &choice.logprobs {
                converted.insert(String::from("logprobs"), logprobs.clone());
            }
            Value::Object(converted)
        })
        .collect();
    let mut body = Map::new();
    body.insert(String::from("id"), Value::from(response.id.as_str()));
    body.insert(String::from("created"), Value::from(created));
    body.insert(
        String::from("model"),
        model.map_or(Value::Null, Value::from),
    );
    body.insert(String::from("object"), Value::from("chat.completion"));
    body.insert(String::from("choices"), Value::Array(choices));
    body.insert(
        String::from("usage"),
        response.usage.as_ref().map_or(Value::Null, usage_value),
    );
    serde_json::to_vec(&without_nulls(Value::Object(body))).map_err(encode_failure)
}

// The text route's chat-deployment answer: LiteLLM's text completion object keeps its null fields
// but writes no system fingerprint, because the conversion never sets one.
pub fn text_answer_body(response: &CompletionResponse) -> Result<Vec<u8>, CompletionFailure> {
    let mut value = serde_json::to_value(response).map_err(encode_failure)?;
    if let Value::Object(fields) = &mut value
        && fields.get("system_fingerprint").is_some_and(Value::is_null)
    {
        fields.remove("system_fingerprint");
    }
    serde_json::to_vec(&value).map_err(encode_failure)
}

// convert_chat_to_text_completion: the text response the completion route answers with.
#[must_use]
pub fn text_completion_response(chat: ChatResponse) -> CompletionResponse {
    let ChatResponse {
        id,
        created,
        model,
        choices,
        usage,
        ..
    } = chat;
    CompletionResponse {
        id,
        object: String::from("text_completion"),
        created: i64::try_from(created).unwrap_or(i64::MAX),
        model: Some(model),
        choices: choices
            .into_iter()
            .map(|choice| CompletionChoice {
                text: choice.message.content,
                index: i64::from(choice.index),
                // the provider logprobs conversion is a no-op in LiteLLM, so the key stays null
                logprobs: None,
                finish_reason: choice.finish_reason,
                extra: Map::new(),
            })
            .collect(),
        usage: Some(CompletionUsage {
            prompt_tokens: i64::try_from(usage.prompt_tokens).unwrap_or(i64::MAX),
            completion_tokens: i64::try_from(usage.completion_tokens).unwrap_or(i64::MAX),
            total_tokens: i64::try_from(usage.total_tokens).unwrap_or(i64::MAX),
            completion_tokens_details: usage
                .completion_tokens_details
                .and_then(|details| serde_json::to_value(details).ok()),
            prompt_tokens_details: usage
                .prompt_tokens_details
                .and_then(|details| serde_json::to_value(details).ok()),
            extra: Map::new(),
        }),
        // the conversion sets no system fingerprint, and the route drops the null for this path
        system_fingerprint: None,
        extra: Map::new(),
    }
}

fn usage_value(usage: &CompletionUsage) -> Value {
    let mut fields = Map::new();
    fields.insert(
        String::from("prompt_tokens"),
        Value::from(usage.prompt_tokens),
    );
    fields.insert(
        String::from("completion_tokens"),
        Value::from(usage.completion_tokens),
    );
    fields.insert(
        String::from("total_tokens"),
        Value::from(usage.total_tokens),
    );
    if let Some(details) = &usage.completion_tokens_details {
        fields.insert(String::from("completion_tokens_details"), details.clone());
    }
    if let Some(details) = &usage.prompt_tokens_details {
        fields.insert(String::from("prompt_tokens_details"), details.clone());
    }
    Value::Object(fields)
}

fn encode_failure(source: serde_json::Error) -> CompletionFailure {
    failure(
        500,
        Some("internal_server_error"),
        &format!("the bridged completion answer could not be written: {source}"),
    )
}
