use serde_json::{Map, Value};

use super::failure;
use crate::chat::response::ChatResponse;
use crate::text_completion_execution::failure::CompletionFailure;
use crate::text_completion_response::{CompletionChoice, CompletionResponse, CompletionUsage};
use crate::text_completion_stream::without_nulls;

// The fine-tuned branch answers the text body itself, model renamed, nulls dropped as pydantic's exclude_none does
pub fn text_completion_body(
    response: &CompletionResponse,
    model: Option<&str>,
) -> Result<Vec<u8>, CompletionFailure> {
    let mut value = serde_json::to_value(response).map_err(|source| encode_failure(&source))?;
    if let Some(model) = model
        && let Value::Object(fields) = &mut value
    {
        fields.insert(String::from("model"), Value::from(model));
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
    let body = serde_json::json!({
        "id": response.id,
        "created": created,
        "model": model,
        "object": "chat.completion",
        "choices": chat_choices(response),
        "usage": response.usage.as_ref().map(usage_value),
    });
    serde_json::to_vec(without_nulls(body)).map_err(|source| encode_failure(&source))
}

fn chat_choices(response: &CompletionResponse) -> Vec<Value> {
    let mut choices = Vec::new();
    for (index, choice) in response.choices.iter().enumerate() {
        choices.push(serde_json::json!({
            "finish_reason": choice.finish_reason.as_deref(),
            "index": index,
            "message": {
                "content": choice.text.as_deref(),
                "role": "assistant",
            },
            "logprobs": choice.logprobs,
        }));
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
    serde_json::json!({
        "prompt_tokens": usage.prompt_tokens,
        "completion_tokens": usage.completion_tokens,
        "total_tokens": usage.total_tokens,
        "completion_tokens_details": usage.completion_tokens_details,
        "prompt_tokens_details": usage.prompt_tokens_details,
    })
}

fn encode_failure(source: &serde_json::Error) -> CompletionFailure {
    failure(
        500,
        Some("internal_server_error"),
        &format!("the bridged completion answer could not be written: {source}"),
    )
}
