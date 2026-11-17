use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Display;

use betterlite_config::sections::Section;
use betterlite_config::sections::openai_credentials::Raw as CredentialsRaw;
use betterlite_config::sections::openai_provider::Raw as ProviderRaw;
use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::decisions::Json;
use betterlite_openai::chat::ChatRequest;
use betterlite_openai::credentials::LiteLlmGlobals;
use betterlite_openai::embedding::EmbeddingRequest;
use betterlite_openai::openai_base_config::{CatalogCapabilities, Headers, Object, OpenAiSettings};
use betterlite_openai::text_completion_policy::TextCompletionSettings;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError};
use betterlite_oracle_helper_api::helper::HelperResult;
use betterlite_provider::openai_dispatch::OpenAiRequest;
use betterlite_provider::openai_dispatch::OpenAiResult;
use betterlite_provider::openai_dispatch::call::{
    ChatCall, Composition, Credentials, EmbeddingCall, OpenAiOperation, TextCall,
};
use betterlite_provider::openai_dispatch::dispatch::dispatch;
use betterlite_provider::openai_dispatch::error::OpenAiProviderError;
use betterlite_provider::openai_dispatch::fixtures::{
    FixtureAnswer, FixtureLogger, FixtureTextSource, TextReply,
};
use betterlite_provider::openai_dispatch::transport::Deps;
use betterlite_provider::operation::{
    ChatCompletion, OperationError, OperationRequest, OperationResult,
};
use betterlite_streaming::stream_chunk_shaping::creation::StreamInstant;
use indexmap::IndexMap;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Map, Value};
use uuid::Uuid;

// The header names the Python case records beside every name the call itself supplied
const KEPT_HEADERS: [&str; 3] = ["authorization", "content-type", "openai-organization"];

const DEFAULT_STATUS: u16 = 200;

pub(super) fn failure<M>(class: &str, message: M) -> Box<HelperError>
where
    M: Display,
{
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_string(),
    })
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Operation {
    Chat,
    Embedding,
    Text,
}

#[derive(Debug, Default, Deserialize)]
pub(super) struct Globals {
    #[serde(default)]
    pub(super) api_base: Option<String>,
    #[serde(default)]
    pub(super) api_key: Option<String>,
    #[serde(default)]
    pub(super) openai_key: Option<String>,
    #[serde(default)]
    pub(super) organization: Option<String>,
}

// One call the Python helper can make, keyed the way LiteLLM reads its own arguments
#[derive(Debug, Deserialize)]
pub(super) struct Case {
    pub(super) case: Operation,
    pub(super) model: String,
    #[serde(default)]
    pub(super) api_base: Option<String>,
    #[serde(default)]
    pub(super) api_key: Option<String>,
    #[serde(default)]
    pub(super) organization: Option<String>,
    #[serde(default)]
    pub(super) headers: Option<Headers>,
    #[serde(default)]
    pub(super) extra_headers: Option<Headers>,
    #[serde(default)]
    pub(super) metadata: Option<Object>,
    #[serde(default)]
    pub(super) environment: BTreeMap<String, String>,
    #[serde(default)]
    pub(super) globals: Globals,
    #[serde(default)]
    pub(super) preview: bool,
    #[serde(default)]
    pub(super) chat_defaults: Option<Object>,
    #[serde(default)]
    pub(super) text_defaults: Option<Object>,
    #[serde(default)]
    pub(super) messages: Vec<Value>,
    #[serde(default)]
    pub(super) optional_params: Object,
    #[serde(default = "default_status")]
    pub(super) status: u16,
    pub(super) reply: Value,
    #[serde(default)]
    pub(super) text_completion: bool,
}

const fn default_status() -> u16 {
    DEFAULT_STATUS
}

// The clock a projected text reply stamps its created field with, fixed so a case can compare it
fn fixed_now() -> StreamInstant {
    StreamInstant {
        seconds: 1_700_000_000,
        timestamp: 1_700_000_000.0,
    }
}

// The identifier a projected text reply names itself with, fixed so a case can compare it
fn fixed_id() -> Uuid {
    Uuid::from_u128(1)
}

pub(super) async fn dispatch_case(case: &Case) -> HelperResult {
    let composition = composition(case)?;
    let operation = operation(case)?;
    let reply = serde_json::to_vec(&case.reply).map_err(|error| failure("ValueError", error))?;
    // the same fixture body answers either transport, and only the branch this operation runs reads it
    let answer = FixtureAnswer::new(case.status, reply.clone());
    let text = FixtureTextSource::new(TextReply::new(case.status, reply));
    let mut logger = FixtureLogger::default();
    let result = dispatch(
        OperationRequest::OpenAi(OpenAiRequest),
        operation,
        &composition,
        Deps {
            answer: &answer,
            text: &text,
            now: fixed_now,
            next_id: fixed_id,
        },
        &mut logger,
    )
    .await;
    let requests = requests_view(case, &answer, &text)?;
    let logs = logs_view(&logger)?;
    match result {
        Ok(outcome) => Ok(serde_json::json!({
            "requests": requests,
            "reply": reply_view(outcome)?,
            "error": Value::Null,
            "logs": logs,
        })),
        Err(error) => Ok(serde_json::json!({
            "requests": requests,
            "reply": Value::Null,
            "error": serde_json::json!({"status": error_status(&error)}),
            "logs": logs,
        })),
    }
}

// The composition is the real config sections over the case's environment, so the flags load the way main.py loads them
fn composition(case: &Case) -> Result<Composition, Box<HelperError>> {
    let credentials = CredentialsRaw::default()
        .convert(&case.environment)
        .or_config_error()?;
    let mut provider = ProviderRaw::default()
        .convert(&case.environment)
        .or_config_error()?;
    provider.enable_preview_features = case.preview;
    Ok(Composition {
        credentials,
        provider,
        globals: LiteLlmGlobals {
            api_base: case.globals.api_base.clone(),
            api_key: case.globals.api_key.clone().map(SecretString::from),
            openai_key: case.globals.openai_key.clone().map(SecretString::from),
            organization: case.globals.organization.clone(),
        },
        // the case never sets litellm.headers, so only the call's own maps name the headers
        globals_headers: None,
        env_api_key: case
            .environment
            .get("OPENAI_API_KEY")
            .filter(|value| !value.is_empty())
            .map(|value| SecretString::from(value.clone())),
        chat_defaults: chat_defaults(case.chat_defaults.as_ref())?,
        text_defaults: text_defaults(case.text_defaults.as_ref())?,
        catalog: CatalogCapabilities {
            chat_models: BTreeSet::new(),
            text_models: BTreeSet::new(),
        },
    })
}

fn chat_defaults(defaults: Option<&Object>) -> Result<OpenAiSettings, Box<HelperError>> {
    defaults.map_or_else(
        || Ok(OpenAiSettings::default()),
        |fields| {
            serde_json::from_value(Value::Object(fields.clone()))
                .map_err(|error| failure("ValueError", error))
        },
    )
}

fn text_defaults(defaults: Option<&Object>) -> Result<TextCompletionSettings, Box<HelperError>> {
    defaults.map_or_else(
        || Ok(TextCompletionSettings::default()),
        |fields| {
            serde_json::from_value(Value::Object(fields.clone()))
                .map_err(|error| failure("ValueError", error))
        },
    )
}

fn explicit_credentials(case: &Case) -> Credentials {
    Credentials {
        api_base: case.api_base.clone(),
        api_key: case.api_key.clone().map(SecretString::from),
        organization: case.organization.clone(),
    }
}

fn operation(case: &Case) -> Result<OpenAiOperation, Box<HelperError>> {
    match case.case {
        Operation::Chat => Ok(OpenAiOperation::Chat(Box::new(chat_call(case)?))),
        Operation::Embedding => Ok(OpenAiOperation::Embedding(Box::new(embedding_call(case)?))),
        Operation::Text => Ok(OpenAiOperation::Text(Box::new(text_call(case)?))),
    }
}

fn chat_call(case: &Case) -> Result<ChatCall, Box<HelperError>> {
    let mut body = case.optional_params.clone();
    body.insert(
        String::from("messages"),
        Value::Array(case.messages.clone()),
    );
    let encoded =
        serde_json::to_vec(&Value::Object(body)).map_err(|error| failure("ValueError", error))?;
    let request = ChatRequest::parse(&encoded).map_err(|error| failure("ValueError", error))?;
    Ok(ChatCall {
        model: case.model.clone(),
        request,
        credentials: explicit_credentials(case),
        headers: case.headers.clone(),
        extra_headers: case.extra_headers.clone(),
        timeout: None,
        metadata: metadata(case.metadata.as_ref())?,
    })
}

fn embedding_call(case: &Case) -> Result<EmbeddingCall, Box<HelperError>> {
    let encoded = serde_json::to_vec(&Value::Object(case.optional_params.clone()))
        .map_err(|error| failure("ValueError", error))?;
    let request =
        EmbeddingRequest::parse(&encoded).map_err(|error| failure("ValueError", error))?;
    Ok(EmbeddingCall {
        model: case.model.clone(),
        request,
        // the route resolved no format of its own, so the body's encoding_format names it
        encoding_format: None,
        credentials: explicit_credentials(case),
        headers: case.headers.clone(),
        timeout: None,
    })
}

fn text_call(case: &Case) -> Result<TextCall, Box<HelperError>> {
    let messages = case
        .messages
        .iter()
        .cloned()
        .map(|message| {
            serde_json::from_value(message).map_err(|error| failure("ValueError", error))
        })
        .collect::<Result<Vec<JsonValue>, Box<HelperError>>>()?;
    Ok(TextCall {
        model: case.model.clone(),
        messages,
        optional_params: case.optional_params.clone(),
        credentials: explicit_credentials(case),
        headers: merged_headers(case),
        timeout: None,
        acompletion: false,
        text_completion: case.text_completion,
    })
}

fn metadata(
    fields: Option<&Object>,
) -> Result<Option<IndexMap<String, JsonValue>>, Box<HelperError>> {
    fields.map_or_else(
        || Ok(None),
        |fields| {
            serde_json::from_value(Value::Object(fields.clone()))
                .map(Some)
                .map_err(|error| failure("ValueError", error))
        },
    )
}

// completion() takes the supplied map or else the extra one, then writes the extra map over it
fn merged_headers(case: &Case) -> Option<Headers> {
    let supplied = case.headers.as_ref().filter(|map| !map.is_empty());
    let base = supplied.or(case.extra_headers.as_ref().filter(|map| !map.is_empty()));
    let mut merged = base.cloned().unwrap_or_default();
    if let Some(extra) = &case.extra_headers {
        merged.extend(extra.clone());
    }
    (!merged.is_empty()).then_some(merged)
}

fn requests_view(
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
        let body =
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

fn logs_view(logger: &FixtureLogger) -> Result<Value, Box<HelperError>> {
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
        Json::Bool(_flag) | Json::Number(_number) | Json::List(_items) | Json::Object(_fields) => {
            Value::String(String::from("object"))
        }
    }
}

fn reply_view(outcome: OperationResult) -> Result<Value, Box<HelperError>> {
    match outcome {
        OperationResult::Chat(ChatCompletion::OpenAi(reply)) => {
            let dump = serde_json::to_value(&reply).map_err(|error| failure("Fixture", error))?;
            Ok(chat_view(&dump))
        }
        OperationResult::Embedding(reply) => {
            let dump = serde_json::to_value(&reply).map_err(|error| failure("Fixture", error))?;
            Ok(embedding_view(&dump))
        }
        OperationResult::OpenAi(OpenAiResult::Text(reply)) => {
            let dump = serde_json::to_value(&reply).map_err(|error| failure("Fixture", error))?;
            Ok(chat_view(&dump))
        }
        OperationResult::OpenAi(OpenAiResult::TextStream(_stream)) => {
            Err(failure("Fixture", "the text branch opened a stream"))
        }
        _other => Err(failure("Fixture", "the dispatch returned another outcome")),
    }
}

fn chat_view(dump: &Value) -> Value {
    let first = dump
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| choices.first())
        .cloned()
        .unwrap_or(Value::Null);
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

fn embedding_view(dump: &Value) -> Value {
    let first = dump
        .get("data")
        .and_then(Value::as_array)
        .and_then(|data| data.first())
        .cloned()
        .unwrap_or(Value::Null);
    serde_json::json!({
        "object": dump.get("object").cloned().unwrap_or(Value::Null),
        "model": dump.get("model").cloned().unwrap_or(Value::Null),
        "embedding": first.get("embedding").cloned().unwrap_or(Value::Null),
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
const fn error_status(error: &OpenAiProviderError) -> Option<u16> {
    match error {
        OpenAiProviderError::Operation(OperationError::Provider { response }) => {
            Some(response.status)
        }
        OpenAiProviderError::UnsupportedOperation
        | OpenAiProviderError::Operation(_other)
        | OpenAiProviderError::Deployment(_deployment)
        | OpenAiProviderError::Text(_text)
        | OpenAiProviderError::Reply { .. }
        | OpenAiProviderError::FieldType { .. }
        | OpenAiProviderError::HeaderName { .. } => None,
    }
}
