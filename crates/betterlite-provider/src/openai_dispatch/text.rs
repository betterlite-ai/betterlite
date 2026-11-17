use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::decisions::{Extra, Json};
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_openai::chat::response::ChatResponse;
use betterlite_openai::openai_base_config::{Headers, Object};
use betterlite_openai::text_completion_policy::reply::to_chat_model_response;
use betterlite_openai::text_completion_policy::{
    TextCompletionSettings, transform_completion_request,
};
use betterlite_openai::text_completion_transport::call::{
    TextCompletionOutcome, TextCompletionRequest,
};
use betterlite_openai::text_completion_transport::logging::SecretHeaders;
use betterlite_openai::text_completion_transport::sdk::ClientConstructor;
use betterlite_openai::text_completion_transport::stream::CompletionStream;
use betterlite_openai::text_completion_transport::{
    TextCompletionConfigs, TextCompletionTransport,
};
use betterlite_streaming::stream_chunk_shaping::creation::StreamInstant;
use secrecy::SecretString;
use serde_json::Number;
use uuid::Uuid;

use crate::operation::OperationError;

use super::call::{Composition, TextCall};
use super::compose::resolve;
use super::error::OpenAiProviderError;
use super::headers::resolved_headers;
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
    pub optional_params: Object,
    pub headers: Option<Headers>,
    pub timeout: f64,
    pub acompletion: bool,
    pub text_completion: bool,
    pub api_type: &'static str,
    pub api_version: Option<&'static str>,
}

pub enum TextOutcome {
    Buffered(JsonValue),
    Stream(Box<CompletionStream>),
}

pub fn plan(composition: &Composition, call: &TextCall) -> Result<TextPlan, OpenAiProviderError> {
    let credentials = resolve(composition, &call.credentials);
    let mut optional_params = call.optional_params.clone();
    merge_defaults(&mut optional_params, &composition.text_defaults)?;
    Ok(TextPlan {
        model: call.model.clone(),
        messages: call.messages.clone(),
        api_base: credentials.api_base,
        api_key: credentials.api_key,
        optional_params,
        headers: resolved_headers(composition, call.headers.as_ref(), None),
        timeout: call.timeout.unwrap_or(DEFAULT_TIMEOUT_SECONDS),
        acompletion: call.acompletion,
        text_completion: call.text_completion,
        api_type: API_TYPE,
        api_version: API_VERSION,
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
    <Source::Constructor as ClientConstructor>::Client: Clone,
    Logger: OpenAiLogger + Send,
{
    let transport = TextCompletionTransport {
        constructor: text.constructor(plan.api_key.as_ref()),
        configs: TRANSFORMS,
        now,
        next_id,
    };
    let request = text_request(plan)?;
    let begun = transport.begin(
        request,
        ProviderId::TextCompletionOpenai,
        logger.completion(),
    )?;
    let sent = transport.send(&begun, None).await?;
    match transport.finish(&begun, sent, logger.completion())? {
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

fn text_request(plan: &TextPlan) -> Result<TextCompletionRequest, OpenAiProviderError> {
    let serialized = serde_json::to_value(&plan.messages)
        .map_err(|source| OpenAiProviderError::Operation(OperationError::Encode { source }))?;
    let messages = serde_json::from_value(serialized)
        .map_err(|source| OpenAiProviderError::Operation(OperationError::Encode { source }))?;
    Ok(TextCompletionRequest {
        model: Some(plan.model.clone()),
        messages: Some(messages),
        api_key: plan.api_key.clone(),
        api_base: Some(plan.api_base.clone()),
        timeout: plan.timeout,
        // main.py assigns the SDK module global only, and B2238's handler builds its client without an organization
        organization: None,
        optional_params: plan.optional_params.clone(),
        headers: plan.headers.clone().map(SecretHeaders),
        acompletion: plan.acompletion,
    })
}

// main.py projects text to chat only for an absent or literally false stream, acompletion and text_completion.
#[must_use]
pub fn projects_to_chat(plan: &TextPlan) -> bool {
    // `is False` compares identity: an absent key took the False default, and a null or 0 stream is not False
    let stream_is_false = matches!(
        plan.optional_params.get("stream"),
        None | Some(serde_json::Value::Bool(false))
    );
    stream_is_false && !plan.acompletion && !plan.text_completion
}

// ModelResponse() before the conversion: the reply's id, created, model and usage replace these, the object stays
fn fresh_target(identifier: &dyn Fn() -> Uuid, clock: &dyn Fn() -> i64) -> Extra {
    let mut target = Extra::new();
    target.insert(String::from("choices"), Json::List(Vec::new()));
    target.insert(String::from("created"), Json::Number(Number::from(clock())));
    target.insert(
        String::from("id"),
        Json::Text(format!("chatcmpl-{}", identifier())),
    );
    target.insert(String::from("model"), Json::Null);
    target.insert(
        String::from("object"),
        Json::Text(String::from("chat.completion")),
    );
    target.insert(String::from("system_fingerprint"), Json::Null);
    target
}

// B2237's convert_to_chat_model_response_object: every text choice becomes one assistant message, indexed by position.
pub fn project(
    response: &JsonValue,
    now: &StreamInstant,
    next_id: fn() -> Uuid,
) -> Result<ChatResponse, OpenAiProviderError> {
    let identifier = || next_id();
    let clock = || now.seconds;
    let serialized = serde_json::to_value(fresh_target(&identifier, &clock))
        .map_err(|source| OpenAiProviderError::Operation(OperationError::Encode { source }))?;
    let target: Object = serde_json::from_value(serialized)
        .map_err(|source| OpenAiProviderError::Operation(OperationError::Encode { source }))?;
    let body = serde_json::to_value(response)
        .map_err(|source| OpenAiProviderError::Operation(OperationError::Encode { source }))?;
    let converted = to_chat_model_response(Some(body), Some(target), &identifier, &clock)?;
    let reply_json = serde_json::to_value(converted)
        .map_err(|source| OpenAiProviderError::Operation(OperationError::Encode { source }))?;
    serde_json::from_value(reply_json).map_err(|source| {
        OpenAiProviderError::Operation(OperationError::Decode {
            operation: "text completion chat projection",
            source,
        })
    })
}
