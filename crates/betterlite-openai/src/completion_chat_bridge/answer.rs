use serde_json::{Map, Value};

use super::failure;
use crate::chat::response::ChatResponse;
use crate::text_completion_execution::failure::CompletionFailure;
use crate::text_completion_response::{CompletionChoice, CompletionResponse, CompletionUsage};
use crate::text_completion_stream::{field, without_nulls};

// The fine-tuned branch answers the text body itself, with the model renamed to the caller's
pub fn text_completion_body(
    response: &CompletionResponse,
    model: Option<&str>,
) -> Result<Vec<u8>, CompletionFailure> {
    let mut value = serde_json::to_value(response).map_err(|source| encode_failure(&source))?;
    if let Some(asked) = model
        && let Value::Object(fields) = &mut value
    {
        fields.insert(String::from("model"), Value::from(asked));
    }
    let answer = without_nulls(value);
    serde_json::to_vec(&answer).map_err(|source| encode_failure(&source))
}

// convert_to_chat_model_response_object: a fresh chat response carrying the text answer, stamped with the call's own time
pub fn chat_completion_body(
    response: &CompletionResponse,
    created: i64,
    model: Option<&str>,
) -> Result<Vec<u8>, CompletionFailure> {
    let fields = vec![
        field("id", Some(Value::from(response.id.as_str()))),
        field("created", Some(Value::from(created))),
        field("object", Some(Value::from("chat.completion"))),
        field("choices", Some(Value::Array(chat_choices(response)))),
        field("model", model.map(Value::from)),
        field("usage", response.usage.as_ref().map(usage_value)),
    ];
    serde_json::to_vec(&without_nulls(Value::Object(Map::from_iter(fields))))
        .map_err(|source| encode_failure(&source))
}

fn chat_choices(response: &CompletionResponse) -> Vec<Value> {
    let mut choices = Vec::new();
    for (index, choice) in response.choices.iter().enumerate() {
        let message = Value::Object(Map::from_iter([
            field("content", choice.text.as_deref().map(Value::from)),
            field("role", Some(Value::from("assistant"))),
        ]));
        let finish = choice.finish_reason.as_deref();
        choices.push(Value::Object(Map::from_iter([
            field("finish_reason", finish.map(Value::from)),
            field("index", Some(Value::from(index))),
            field("message", Some(message)),
            field("logprobs", choice.logprobs.clone()),
        ])));
    }
    choices
}

// The text route's chat-deployment answer keeps its null fields but writes no system fingerprint
pub fn text_answer_body(response: &CompletionResponse) -> Result<Vec<u8>, CompletionFailure> {
    let mut value = serde_json::to_value(response).map_err(|source| encode_failure(&source))?;
    if let Value::Object(fields) = &mut value
        && fields.get("system_fingerprint").is_some_and(Value::is_null)
    {
        fields.remove("system_fingerprint");
    }
    serde_json::to_vec(&value).map_err(|source| encode_failure(&source))
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
    let completion = usage.completion_tokens_details.clone();
    let prompt = usage.prompt_tokens_details.clone();
    let counted = Some(Value::from(usage.completion_tokens));
    Value::Object(Map::from_iter([
        field("prompt_tokens", Some(Value::from(usage.prompt_tokens))),
        field("completion_tokens", counted),
        field("total_tokens", Some(Value::from(usage.total_tokens))),
        field("completion_tokens_details", completion),
        field("prompt_tokens_details", prompt),
    ]))
}

fn encode_failure(source: &serde_json::Error) -> CompletionFailure {
    failure(
        500,
        Some("internal_server_error"),
        &format!("the bridged completion answer could not be written: {source}"),
    )
}
