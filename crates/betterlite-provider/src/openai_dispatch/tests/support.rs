use std::collections::BTreeSet;

use betterlite_config::sections::openai_credentials::Settings as CredentialsSettings;
use betterlite_config::sections::openai_provider::Settings as ProviderSettings;
use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::decisions::Json;
use betterlite_openai::chat::ChatRequest;
use betterlite_openai::credentials::LiteLlmGlobals;
use betterlite_openai::embedding::EmbeddingRequest;
use betterlite_openai::openai_base_config::{CatalogCapabilities, Object, OpenAiSettings};
use betterlite_openai::text_completion_policy::TextCompletionSettings;
use betterlite_streaming::stream_chunk_shaping::creation::StreamInstant;
use indexmap::IndexMap;
use secrecy::SecretString;
use serde_json::Number;
use uuid::Uuid;

use crate::openai_dispatch::OpenAiRequest;
use crate::openai_dispatch::call::Credentials;
use crate::openai_dispatch::call::{Composition, OpenAiOperation};
use crate::openai_dispatch::dispatch::dispatch;
use crate::openai_dispatch::error::OpenAiProviderError;
use crate::openai_dispatch::fixtures::answer::FixtureAnswer;
use crate::openai_dispatch::fixtures::logger::FixtureLogger;
use crate::openai_dispatch::fixtures::text_transport::FixtureTextSource;
use crate::openai_dispatch::logging::LoggedCall;
use crate::openai_dispatch::transport::Deps;
use crate::operation::{OperationRequest, OperationResult};

pub(crate) const CHAT_REPLY: &str = r#"{"id":"chatcmpl-1","object":"chat.completion","created":1700000000,"model":"gpt-4o-2024-08-06","choices":[{"index":0,"message":{"role":"assistant","content":"hi"},"logprobs":null,"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}"#;
pub(crate) const EMBEDDING_REPLY: &str = r#"{"object":"list","data":[{"object":"embedding","index":0,"embedding":[0.25,0.5]}],"model":"text-embedding-3-small","usage":{"prompt_tokens":2,"total_tokens":2}}"#;
pub(crate) const TEXT_REPLY: &str = r#"{"id":"cmpl-1","object":"text_completion","created":1700000000,"model":"gpt-3.5-turbo-instruct","choices":[{"index":0,"text":"hello","finish_reason":"stop","logprobs":null}],"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}"#;

pub(crate) fn composition() -> Composition {
    Composition {
        credentials: CredentialsSettings::default(),
        provider: ProviderSettings::default(),
        embedding_format: None,
        globals: LiteLlmGlobals::default(),
        globals_headers: None,
        env_api_key: None,
        chat_defaults: OpenAiSettings::default(),
        text_defaults: TextCompletionSettings::default(),
        catalog: CatalogCapabilities {
            chat_models: BTreeSet::new(),
            text_models: BTreeSet::new(),
        },
    }
}

pub(crate) fn credentials(
    api_base: Option<&str>,
    api_key: Option<&str>,
    organization: Option<&str>,
) -> Credentials {
    Credentials {
        api_base: api_base.map(str::to_owned),
        api_key: api_key.map(SecretString::from),
        organization: organization.map(str::to_owned),
    }
}

// litellm's globals carry the same identity as a call's explicit credentials, beside litellm.openai_key
pub(crate) fn globals(identity: Credentials, openai_key: Option<&str>) -> LiteLlmGlobals {
    LiteLlmGlobals {
        api_base: identity.api_base,
        api_key: identity.api_key,
        openai_key: openai_key.map(SecretString::from),
        organization: identity.organization,
    }
}

// What main.py logged under additional_args.headers, read back by name.
pub(crate) fn logged_header<'call>(call: &'call LoggedCall, name: &str) -> Option<&'call str> {
    let headers = call.headers.as_ref()?;
    match headers.get(name)? {
        Json::Text(value) => Some(value),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => None,
    }
}

pub(crate) fn chat_request(body: &str) -> ChatRequest {
    ChatRequest::parse(body.as_bytes()).expect("a chat request the route accepts")
}

// A request a route built before the parse step, so a test can reach the maps that parse would refuse.
pub(crate) fn raw_chat_request(body: &str) -> ChatRequest {
    serde_json::from_str(body).expect("a chat request body")
}

pub(crate) fn embedding_request(body: &str) -> EmbeddingRequest {
    EmbeddingRequest::parse(body.as_bytes()).expect("an embedding request the route accepts")
}

pub(crate) fn chat_defaults(body: &str) -> OpenAiSettings {
    serde_json::from_str(body).expect("chat defaults")
}

pub(crate) fn text_defaults(body: &str) -> TextCompletionSettings {
    serde_json::from_str(body).expect("text defaults")
}

pub(crate) fn optional_params(body: &str) -> Object {
    serde_json::from_str(body).expect("text optional parameters")
}

pub(crate) fn messages(body: &str) -> Vec<JsonValue> {
    serde_json::from_str(body).expect("messages")
}

pub(crate) fn body_fields(bytes: &[u8]) -> IndexMap<String, JsonValue> {
    serde_json::from_slice(bytes).expect("a JSON object body")
}

pub(crate) fn text_value(value: &str) -> JsonValue {
    JsonValue::Text(value.to_owned())
}

pub(crate) fn number_value(value: i64) -> JsonValue {
    JsonValue::Number(Number::from(value))
}

pub(crate) fn real_value(value: f64) -> JsonValue {
    JsonValue::Number(Number::from_f64(value).expect("a finite number"))
}

// A body or a settings map an SDK received, read back as the route's own JSON value type.
pub(crate) fn fields_of(object: &Object) -> IndexMap<String, JsonValue> {
    let bytes = serde_json::to_vec(object).expect("a serializable object");
    serde_json::from_slice(&bytes).expect("a JSON object")
}

pub(crate) fn field<'body>(
    fields: &'body IndexMap<String, JsonValue>,
    name: &str,
) -> &'body JsonValue {
    fields
        .get(name)
        .unwrap_or_else(|| panic!("the body has {name}"))
}

// The clock a projected text reply stamps its created field with, fixed so a test can compare it
pub(crate) fn fixed_now() -> StreamInstant {
    StreamInstant {
        seconds: 1_700_000_000,
        timestamp: 1_700_000_000.0,
    }
}

// The identifier a projected text reply names itself with, fixed so a test can compare it
pub(crate) fn fixed_id() -> Uuid {
    Uuid::from_u128(1)
}

async fn run(
    composition: &Composition,
    operation: OpenAiOperation,
    answer: &FixtureAnswer,
    text: &FixtureTextSource,
    logger: &mut FixtureLogger,
) -> Result<OperationResult, OpenAiProviderError> {
    dispatch(
        OperationRequest::OpenAi(OpenAiRequest),
        operation,
        composition,
        Deps {
            answer,
            text,
            now: fixed_now,
            next_id: fixed_id,
        },
        logger,
    )
    .await
}

mod dispatch_chat;
mod dispatch_embedding;
mod dispatch_text;

pub(crate) use dispatch_chat::dispatch_chat;
pub(crate) use dispatch_embedding::dispatch_embedding;
pub(crate) use dispatch_text::dispatch_text;
