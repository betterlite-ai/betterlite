use std::collections::BTreeSet;

use betterlite_models::decisions::Json;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_provider::openai_dispatch::OpenAiResult;
use betterlite_provider::openai_dispatch::error::OpenAiProviderError;
use betterlite_provider::openai_dispatch::fixtures::answer::FixtureAnswer;
use betterlite_provider::openai_dispatch::fixtures::logger::FixtureLogger;
use betterlite_provider::openai_dispatch::fixtures::text_transport::FixtureTextSource;
use betterlite_provider::operation::{ChatCompletion, OperationError, OperationResult};
use serde_json::{Map, Value};

use super::case::{Case, Operation, failure};

// The header names the Python case records beside every name the call itself supplied
const KEPT_HEADERS: [&str; 3] = ["authorization", "content-type", "openai-organization"];

pub(super) fn requests_view(
    case: &Case,
    answer: &FixtureAnswer,
    text: &FixtureTextSource,
) -> Result<Value, Box<HelperError>> {
    match case.case {
        Operation::Text => text_requests(text),
        Operation::Chat | Operation::Embedding => answer_requests(case, answer),
    }
}

fn answer_requests(case: &Case, answer: &FixtureAnswer) -> Result<Value, Box<HelperError>> {
    let kept = kept_headers(case);
    let requests = answer.requests();
    let mut rendered = Vec::with_capacity(requests.len());
    for request in &requests {
        let body: Value =
            serde_json::from_slice(&request.body).map_err(|error| failure("Fixture", error))?;
        let headers: Map<String, Value> = request
            .headers
            .iter()
            .filter(|(name, _value)| kept.contains(name.as_str()))
            .map(|(name, value)| (name.clone(), Value::String(value.clone())))
            .collect();
        rendered.push(serde_json::json!({
            "method": request.method.clone(),
            "url": request.url.clone(),
            "body": body,
            "headers": headers,
        }));
    }
    Ok(Value::Array(rendered))
}

// The text transport posts to its base URL's completions path; the case compares the composed body only
fn text_requests(text: &FixtureTextSource) -> Result<Value, Box<HelperError>> {
    let attempts = text.attempts();
    let bodies = text.bodies();
    let mut rendered = Vec::with_capacity(bodies.len());
    for (attempt, body) in attempts.iter().zip(bodies) {
        let base = attempt.base_url.clone().unwrap_or_default();
        rendered.push(serde_json::json!({
            "method": "POST",
            "url": format!("{}/completions", base.trim_end_matches('/')),
            "body": Value::Object(body),
        }));
    }
    Ok(Value::Array(rendered))
}

// The headers the Python case keeps: its own fixed set plus every name the case itself supplied
fn kept_headers(case: &Case) -> BTreeSet<String> {
    let mut kept: BTreeSet<String> = KEPT_HEADERS.iter().map(|name| (*name).to_owned()).collect();
    for name in case.headers.iter().flat_map(|map| map.keys()) {
        kept.insert(name.to_lowercase());
    }
    for name in case.extra_headers.iter().flat_map(|map| map.keys()) {
        kept.insert(name.to_lowercase());
    }
    kept
}

pub(super) fn logs_view(logger: &FixtureLogger) -> Result<Value, Box<HelperError>> {
    let mut rendered = Vec::with_capacity(logger.calls.len());
    for call in &logger.calls {
        let input = serde_json::to_value(&call.input).map_err(|error| failure("Fixture", error))?;
        let headers = match &call.headers {
            Some(headers) => {
                serde_json::to_value(headers).map_err(|error| failure("Fixture", error))?
            }
            None => Value::Null,
        };
        rendered.push(serde_json::json!({
            "input": input,
            "headers": headers,
            "original_response": original_kind(&call.original_response),
        }));
    }
    Ok(Value::Array(rendered))
}

// The Python recorder names the logged answer by kind, never by its text
fn original_kind(response: &Json) -> Value {
    match response {
        Json::Text(_text) => Value::String(String::from("text")),
        Json::Null => Value::Null,
        Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
            Value::String(String::from("object"))
        }
    }
}

// The three shapes the OpenAI request reaches; every other outcome is a fixture mistake, so each variant is named.
#[rustfmt::skip]
pub(super) fn reply_view(outcome: OperationResult) -> Result<Value, Box<HelperError>> {
    match outcome {
        OperationResult::Chat(ChatCompletion::OpenAi(reply)) => {
            let dump = serde_json::to_value(&reply).map_err(|error| failure("Fixture", error))?;
            Ok(chat_view(&dump))
        }
        OperationResult::Embedding(reply) => {
            let dump = serde_json::to_value(&reply).map_err(|error| failure("Fixture", error))?;
            let first = first_entry(&dump, "data");
            Ok(serde_json::json!({
                "object": dump.get("object").cloned().unwrap_or(Value::Null),
                "model": dump.get("model").cloned().unwrap_or(Value::Null),
                "embedding": first.get("embedding").cloned().unwrap_or(Value::Null),
                "total_tokens": total_tokens(&dump),
            }))
        }
        OperationResult::OpenAi(OpenAiResult::Text(reply)) => {
            let dump = serde_json::to_value(&reply).map_err(|error| failure("Fixture", error))?;
            Ok(chat_view(&dump))
        }
        OperationResult::OpenAi(OpenAiResult::TextStream(_stream)) => Err(failure("Fixture", "the text branch opened a stream")),
        OperationResult::Chat(ChatCompletion::AiohttpOpenAi(_)) | OperationResult::Stream(_) | OperationResult::AiohttpOpenAi(_)
        | OperationResult::Azure(_) | OperationResult::AzureAi(_) | OperationResult::Anthropic(_) | OperationResult::Bedrock(_)
        | OperationResult::VertexAi(_) | OperationResult::Gemini(_) | OperationResult::XaiMistral(_) | OperationResult::EmbeddingRerank(_)
        | OperationResult::Compatible(_) | OperationResult::Legacy(_) | OperationResult::SelfHosted(_) | OperationResult::Image(_)
        | OperationResult::Rerank(_) | OperationResult::File(_) | OperationResult::Batch(_) | OperationResult::Responses(_)
        | OperationResult::Transcription(_) | OperationResult::Speech(_) | OperationResult::Messages(_) | OperationResult::OpenAiMedia(_)
        | OperationResult::OpenAiResource(_) | OperationResult::OpenAiAssistant(_) | OperationResult::AzureMedia(_) | OperationResult::AzureResource(_)
        | OperationResult::FoundryResource(_) | OperationResult::BedrockChat(_) | OperationResult::BedrockOther(_) | OperationResult::BedrockServices(_)
        | OperationResult::BedrockImage(_) | OperationResult::BedrockAgent(_) | OperationResult::BedrockInteractive(_) | OperationResult::VertexPartner(_)
        | OperationResult::VertexMedia(_) | OperationResult::VertexService(_) | OperationResult::VertexEmbedding(_) => Err(failure("Fixture", "the dispatch returned another outcome")),
    }
}

fn first_entry(dump: &Value, field: &str) -> Value {
    dump.get(field)
        .and_then(Value::as_array)
        .and_then(|entries| entries.first())
        .cloned()
        .unwrap_or(Value::Null)
}

fn chat_view(dump: &Value) -> Value {
    let first = first_entry(dump, "choices");
    let content = match first.get("message") {
        Some(Value::Object(message)) => message.get("content").cloned().unwrap_or(Value::Null),
        _other => first.get("text").cloned().unwrap_or(Value::Null),
    };
    serde_json::json!({
        "id": dump.get("id").cloned().unwrap_or(Value::Null),
        "object": dump.get("object").cloned().unwrap_or(Value::Null),
        "model": dump.get("model").cloned().unwrap_or(Value::Null),
        "content": content,
        "finish_reason": first.get("finish_reason").cloned().unwrap_or(Value::Null),
        "total_tokens": total_tokens(dump),
    })
}

fn total_tokens(dump: &Value) -> Value {
    dump.get("usage")
        .and_then(|usage| usage.get("total_tokens"))
        .cloned()
        .unwrap_or(Value::Null)
}

// LiteLLM answers a status of 400 and up as the provider's own error, so the status is the one compared
pub(super) const fn error_status(error: &OpenAiProviderError) -> Option<u16> {
    match error {
        OpenAiProviderError::Operation(OperationError::Provider { response }) => {
            Some(response.status)
        }
        OpenAiProviderError::UnsupportedOperation
        | OpenAiProviderError::Operation(_)
        | OpenAiProviderError::Deployment(_)
        | OpenAiProviderError::Text(_)
        | OpenAiProviderError::Reply { .. }
        | OpenAiProviderError::FieldType { .. }
        | OpenAiProviderError::HeaderName { .. } => None,
    }
}
