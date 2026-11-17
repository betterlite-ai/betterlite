use std::collections::BTreeMap;

use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_openai::chat::response::{AssistantMessage, ChatChoice, ChatResponse, ChatUsage};
use betterlite_openai::openai_base_config::{Headers, Object};
use betterlite_openai::text_completion_policy::{
    TextCompletionSettings, transform_completion_request,
};
use betterlite_openai::text_completion_transport::call::{
    TextCompletionOutcome, TextCompletionRequest,
};
use betterlite_openai::text_completion_transport::logging::SecretHeaders;
use betterlite_openai::text_completion_transport::stream::CompletionStream;
use betterlite_openai::text_completion_transport::{
    TextCompletionConfigs, TextCompletionTransport,
};
use betterlite_streaming::stream_chunk_shaping::creation::StreamInstant;
use secrecy::SecretString;
use uuid::Uuid;

use crate::operation::OperationError;

use super::call::{Composition, TextCall};
use super::compose::resolve;
use super::error::OpenAiProviderError;
use super::headers::{present_text, resolved_headers};
use super::logging::OpenAiLogger;
use super::transport::TextClientSource;

// LiteLLM's request timeout, which a call without its own timeout uses
const DEFAULT_TIMEOUT_SECONDS: f64 = 6000.0;

// main.py sets the SDK module globals below; betterlite carries them per call and no module state is written.
const API_TYPE: &str = "openai";
const API_VERSION: Option<&str> = None;

// main.py's text completion switch reaches these four provider tags, all sharing the one transform.
const TRANSFORMS: TextCompletionConfigs = TextCompletionConfigs {
    open_ai: transform_completion_request,
    fireworks_ai: transform_completion_request,
    together_ai: transform_completion_request,
    text_completion_inception: transform_completion_request,
};

// What the text branch resolved before it builds the transport request.
pub struct TextPlan {
    pub model: String,
    pub messages: Vec<JsonValue>,
    pub api_base: String,
    pub api_key: Option<SecretString>,
    // main.py sets the SDK organization only from a truthy global, and no explicit or environment lookup runs here
    pub organization: Option<String>,
    pub optional_params: Object,
    pub headers: Option<Headers>,
    pub timeout: f64,
    pub acompletion: bool,
    pub text_completion: bool,
    pub api_type: &'static str,
    pub api_version: Option<&'static str>,
    pub created: u64,
}

pub enum TextOutcome {
    Buffered(JsonValue),
    Stream(Box<CompletionStream>),
}

pub fn plan(
    composition: &Composition,
    call: &TextCall,
    now: StreamInstant,
) -> Result<TextPlan, OpenAiProviderError> {
    let credentials = resolve(composition, &call.credentials);
    let mut optional_params = call.optional_params.clone();
    merge_defaults(&mut optional_params, &composition.text_defaults)?;
    Ok(TextPlan {
        model: call.model.clone(),
        messages: call.messages.clone(),
        api_base: credentials.api_base,
        api_key: credentials.api_key,
        organization: present_text(composition.globals.organization.as_deref()).map(str::to_owned),
        optional_params,
        headers: resolved_headers(composition, call.headers.as_ref(), None),
        timeout: call.timeout.unwrap_or(DEFAULT_TIMEOUT_SECONDS),
        acompletion: call.acompletion,
        text_completion: call.text_completion,
        api_type: API_TYPE,
        api_version: API_VERSION,
        created: u64::try_from(now.seconds).unwrap_or_default(),
    })
}

// main.py merges the text config only where the call left the key absent.
fn merge_defaults(
    optional_params: &mut Object,
    defaults: &TextCompletionSettings,
) -> Result<(), OpenAiProviderError> {
    let serialized = serde_json::to_value(defaults)
        .map_err(|source| OpenAiProviderError::Operation(OperationError::Encode { source }))?;
    let fields: Object = serde_json::from_value(serialized)
        .map_err(|source| OpenAiProviderError::Operation(OperationError::Encode { source }))?;
    for (name, value) in fields {
        optional_params.entry(name).or_insert(value);
    }
    Ok(())
}

pub async fn execute<Source, Logger>(
    plan: &TextPlan,
    text: &Source,
    now: fn() -> StreamInstant,
    next_id: fn() -> Uuid,
    logger: &mut Logger,
) -> Result<TextOutcome, OpenAiProviderError>
where
    Source: TextClientSource,
    Source::Constructor::Client: Clone,
    Logger: OpenAiLogger,
{
    let transport = TextCompletionTransport {
        constructor: text.constructor(plan.api_key.as_ref()),
        configs: TRANSFORMS,
        now,
        next_id,
    };
    let messages = serde_json::to_value(&plan.messages)
        .map_err(|source| OpenAiProviderError::Operation(OperationError::Encode { source }))?;
    let messages = serde_json::from_value(messages)
        .map_err(|source| OpenAiProviderError::Operation(OperationError::Encode { source }))?;
    let request = TextCompletionRequest {
        model: Some(plan.model.clone()),
        messages: Some(messages),
        api_key: plan.api_key.clone(),
        api_base: Some(plan.api_base.clone()),
        timeout: plan.timeout,
        organization: plan.organization.clone(),
        optional_params: plan.optional_params.clone(),
        headers: plan.headers.clone().map(SecretHeaders),
        acompletion: plan.acompletion,
    };
    let completion = logger.completion();
    let begun = transport.begin(request, ProviderId::TextCompletionOpenai, completion)?;
    let sent = transport.send(&begun, None).await?;
    match transport.finish(&begun, sent, completion)? {
        TextCompletionOutcome::Buffered(reply) => {
            let response = serde_json::from_value(reply.response).map_err(|source| {
                OpenAiProviderError::Operation(OperationError::Decode {
                    operation: "text completion",
                    source,
                })
            })?;
            Ok(TextOutcome::Buffered(response))
        }
        TextCompletionOutcome::Stream(stream) => Ok(TextOutcome::Stream(Box::new(stream))),
    }
}

// main.py projects text to chat only for an absent or literally false stream, acompletion and text_completion.
#[must_use]
pub fn projects_to_chat(plan: &TextPlan) -> bool {
    // `is False` compares identity: an absent key took the False default, and a null or 0 stream is not False
    let stream_is_false = match plan.optional_params.get("stream") {
        None | Some(JsonValue::Bool(false)) => true,
        Some(_other) => false,
    };
    stream_is_false && !plan.acompletion && !plan.text_completion
}

// main.py logs text success for a truthy stream or an async call, and nothing else.
#[must_use]
pub fn logs_post_call(plan: &TextPlan) -> bool {
    truthy(plan.optional_params.get("stream")) || plan.acompletion
}

// Python truthiness of the stream option; the typed request policy already refused any other shape.
fn truthy(value: Option<&JsonValue>) -> bool {
    match value {
        None | Some(JsonValue::Null | JsonValue::Bool(false)) => false,
        Some(
            JsonValue::Bool(true)
            | JsonValue::Number(_)
            | JsonValue::Text(_)
            | JsonValue::Array(_)
            | JsonValue::Object(_),
        ) => true,
    }
}

// convert_to_chat_model_response_object: each text choice becomes one assistant message, indexed by position.
pub fn project(plan: &TextPlan, response: &JsonValue) -> Result<ChatResponse, OpenAiProviderError> {
    let JsonValue::Object(fields) = response else {
        return Err(field_type("text completion", "an object"));
    };
    let Some(JsonValue::Array(choices)) = fields.get("choices") else {
        return Err(field_type("choices", "an array"));
    };
    let mut projected = Vec::with_capacity(choices.len());
    for (position, choice) in choices.iter().enumerate() {
        projected.push(choice_at(position, choice)?);
    }
    Ok(ChatResponse {
        id: text_at(fields, "id").unwrap_or_default(),
        object: String::from("chat.completion"),
        created: plan.created,
        model: text_at(fields, "model").unwrap_or_else(|| plan.model.clone()),
        choices: projected,
        usage: usage(fields.get("usage")),
        system_fingerprint: None,
        service_tier: None,
    })
}

fn choice_at(position: usize, choice: &JsonValue) -> Result<ChatChoice, OpenAiProviderError> {
    let JsonValue::Object(choice) = choice else {
        return Err(field_type("choices", "an array of objects"));
    };
    let logprobs = match choice.get("logprobs") {
        Some(value) if !matches!(value, JsonValue::Null) => {
            Some(serde_json::to_value(value).map_err(|source| {
                OpenAiProviderError::Operation(OperationError::Encode { source })
            })?)
        }
        _ => None,
    };
    Ok(ChatChoice {
        index: u32::try_from(position).unwrap_or(u32::MAX),
        message: AssistantMessage {
            role: String::from("assistant"),
            // Choices carries the choice's own text, and the SDK's reply normalizes a missing one to null
            content: text_at(choice, "text"),
            tool_calls: None,
            annotations: None,
            audio: None,
            refusal: None,
            provider_specific_fields: Default::default(),
        },
        finish_reason: text_at(choice, "finish_reason"),
        logprobs,
        provider_specific_fields: BTreeMap::new(),
    })
}

fn text_at(fields: &indexmap::IndexMap<String, JsonValue>, name: &str) -> Option<String> {
    match fields.get(name) {
        Some(JsonValue::Text(text)) => Some(text.clone()),
        _ => None,
    }
}

fn usage(raw: Option<&JsonValue>) -> ChatUsage {
    let Some(JsonValue::Object(fields)) = raw else {
        return ChatUsage::default();
    };
    ChatUsage {
        prompt_tokens: count(fields.get("prompt_tokens")),
        completion_tokens: count(fields.get("completion_tokens")),
        total_tokens: count(fields.get("total_tokens")),
        prompt_tokens_details: None,
        completion_tokens_details: None,
    }
}

fn count(value: Option<&JsonValue>) -> u64 {
    match value {
        Some(JsonValue::Number(number)) => number
            .as_i64()
            .and_then(|value| u64::try_from(value).ok())
            .unwrap_or_default(),
        _ => 0,
    }
}

fn field_type(field: &'static str, expected: &'static str) -> OpenAiProviderError {
    OpenAiProviderError::FieldType { field, expected }
}
