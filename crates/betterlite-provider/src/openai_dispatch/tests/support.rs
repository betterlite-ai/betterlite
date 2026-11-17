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
use uuid::Uuid;

use crate::openai_dispatch::OpenAiRequest;
use crate::openai_dispatch::call::Credentials;
use crate::openai_dispatch::call::{
    ChatCall, Composition, EmbeddingCall, OpenAiOperation, TextCall,
};
use crate::openai_dispatch::dispatch::dispatch;
use crate::openai_dispatch::error::OpenAiProviderError;
use crate::openai_dispatch::fixtures::answer::FixtureAnswer;
use crate::openai_dispatch::fixtures::logger::FixtureLogger;
use crate::openai_dispatch::fixtures::text_transport::{FixtureTextSource, TextReply};
use crate::openai_dispatch::logging::LoggedCall;
use crate::openai_dispatch::transport::Deps;
use crate::operation::{OperationRequest, OperationResult};

pub const CHAT_REPLY: &str = r#"{"id":"chatcmpl-1","object":"chat.completion","created":1700000000,"model":"gpt-4o-2024-08-06","choices":[{"index":0,"message":{"role":"assistant","content":"hi"},"logprobs":null,"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}"#;
pub const EMBEDDING_REPLY: &str = r#"{"object":"list","data":[{"object":"embedding","index":0,"embedding":[0.25,0.5]}],"model":"text-embedding-3-small","usage":{"prompt_tokens":2,"total_tokens":2}}"#;
pub const TEXT_REPLY: &str = r#"{"id":"cmpl-1","object":"text_completion","created":1700000000,"model":"gpt-3.5-turbo-instruct","choices":[{"index":0,"text":"hello","finish_reason":"stop","logprobs":null}],"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}"#;

pub fn composition() -> Composition {
    Composition {
        credentials: CredentialsSettings::default(),
        provider: ProviderSettings::default(),
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

pub fn globals(
    api_base: Option<&str>,
    api_key: Option<&str>,
    openai_key: Option<&str>,
    organization: Option<&str>,
) -> LiteLlmGlobals {
    LiteLlmGlobals {
        api_base: api_base.map(str::to_owned),
        api_key: api_key.map(SecretString::from),
        openai_key: openai_key.map(SecretString::from),
        organization: organization.map(str::to_owned),
    }
}

pub fn credentials(
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

// What main.py logged under additional_args.headers, read back by name.
pub fn logged_header<'call>(call: &'call LoggedCall, name: &str) -> Option<&'call str> {
    let headers = call.headers.as_ref()?;
    match headers.get(name)? {
        Json::Text(value) => Some(value),
        _other => None,
    }
}

pub fn chat_request(body: &str) -> ChatRequest {
    ChatRequest::parse(body.as_bytes()).expect("a chat request the route accepts")
}

// A request a route built before the parse step, so a test can reach the maps that parse would refuse.
pub fn raw_chat_request(body: &str) -> ChatRequest {
    serde_json::from_str(body).expect("a chat request body")
}

pub fn embedding_request(body: &str) -> EmbeddingRequest {
    EmbeddingRequest::parse(body.as_bytes()).expect("an embedding request the route accepts")
}

pub fn chat_defaults(body: &str) -> OpenAiSettings {
    serde_json::from_str(body).expect("chat defaults")
}

pub fn text_defaults(body: &str) -> TextCompletionSettings {
    serde_json::from_str(body).expect("text defaults")
}

pub fn optional_params(body: &str) -> Object {
    serde_json::from_str(body).expect("text optional parameters")
}

pub fn messages(body: &str) -> Vec<JsonValue> {
    serde_json::from_str(body).expect("messages")
}

pub fn body_fields(bytes: &[u8]) -> IndexMap<String, JsonValue> {
    serde_json::from_slice(bytes).expect("a JSON object body")
}

pub fn text_value(value: &str) -> JsonValue {
    JsonValue::Text(value.to_owned())
}

pub fn number_value(value: i64) -> JsonValue {
    JsonValue::Number(serde_json::Number::from(value))
}

pub fn field<'body>(fields: &'body IndexMap<String, JsonValue>, name: &str) -> &'body JsonValue {
    fields
        .get(name)
        .unwrap_or_else(|| panic!("the body has {name}"))
}

pub fn object_field<'body>(
    fields: &'body IndexMap<String, JsonValue>,
    name: &str,
) -> &'body IndexMap<String, JsonValue> {
    match field(fields, name) {
        JsonValue::Object(nested) => nested,
        _other => panic!("{name} is an object"),
    }
}

// The clock a projected text reply stamps its created field with, fixed so a test can compare it
pub fn fixed_now() -> StreamInstant {
    StreamInstant {
        seconds: 1_700_000_000,
        timestamp: 1_700_000_000.0,
    }
}

// The identifier a projected text reply names itself with, fixed so a test can compare it
pub fn fixed_id() -> Uuid {
    Uuid::from_u128(1)
}

pub async fn dispatch_chat(
    composition: &Composition,
    call: ChatCall,
    answer: &FixtureAnswer,
    logger: &mut FixtureLogger,
) -> Result<OperationResult, OpenAiProviderError> {
    let text = FixtureTextSource::new(TextReply::new(200, Vec::new()));
    dispatch(
        OperationRequest::OpenAi(OpenAiRequest),
        OpenAiOperation::Chat(Box::new(call)),
        composition,
        Deps {
            answer,
            text: &text,
            now: fixed_now,
            next_id: fixed_id,
        },
        logger,
    )
    .await
}

pub async fn dispatch_embedding(
    composition: &Composition,
    call: EmbeddingCall,
    answer: &FixtureAnswer,
) -> Result<OperationResult, OpenAiProviderError> {
    let text = FixtureTextSource::new(TextReply::new(200, Vec::new()));
    let mut logger = FixtureLogger::default();
    dispatch(
        OperationRequest::OpenAi(OpenAiRequest),
        OpenAiOperation::Embedding(Box::new(call)),
        composition,
        Deps {
            answer,
            text: &text,
            now: fixed_now,
            next_id: fixed_id,
        },
        &mut logger,
    )
    .await
}

pub async fn dispatch_text(
    composition: &Composition,
    call: TextCall,
    text: &FixtureTextSource,
    logger: &mut FixtureLogger,
) -> Result<OperationResult, OpenAiProviderError> {
    let answer = FixtureAnswer::new(200, Vec::new());
    dispatch(
        OperationRequest::OpenAi(OpenAiRequest),
        OpenAiOperation::Text(Box::new(call)),
        composition,
        Deps {
            answer: &answer,
            text,
            now: fixed_now,
            next_id: fixed_id,
        },
        logger,
    )
    .await
}
