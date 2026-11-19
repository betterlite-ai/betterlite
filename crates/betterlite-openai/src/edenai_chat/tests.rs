#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use betterlite_logging::logging_pre_post::args::RawBody;
use betterlite_logging::logging_pre_post::env::{
    DebugSettings, GlobalFlags, LogRecord, LogSink, PrePostEnv,
};
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::decisions::Json as HiddenJson;
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::DateTime;
use indexmap::IndexMap;
use rstest::rstest;
use secrecy::SecretString;
use serde_json::{Number, Value, json};
use thiserror::Error;
use uuid::Uuid;

use super::{
    CatalogReply, ChatReply, EdenAiChatError, ReasoningLookup, ReplyRequest, TransformInput,
    chunk_parser, get_api_base, get_api_key, get_models, models_url, supported_params,
    transform_request, transform_response,
};
use crate::chat::{ChatMessage, MessageContent, Role};
use crate::chat_choices::stream::StreamError;
use crate::chat_content::{Mode, RefuseDownloads};
use crate::compatible_wire::{LogInput, ReplyOptions};
use crate::edenai_common::{EDENAI_API_BASE, EdenAiEnv, RESPONSE_COST_HEADER};
use crate::openai_base_config::{CatalogCapabilities, Headers};

const INIT: &str = r#"{"model":"gpt-4o","call_type":"completion","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#;

struct Silent;

impl LogSink for Silent {
    fn emit(&self, _record: LogRecord) {}
}

fn fixture(redactor: &Redactor) -> (LoggingState, PrePostEnv<'_>) {
    let state = LoggingState::new(
        serde_json::from_str::<LoggingInit>(INIT).unwrap(),
        CorrelationSetup {
            context: &mut CorrelationContext::default(),
            settings: CorrelationSettings::default(),
        },
        redactor,
        || String::from("trace"),
        Duration::default,
    )
    .unwrap();
    let env = PrePostEnv {
        debug: DebugSettings::default(),
        globals: GlobalFlags::default(),
        decision: Decision {
            content: false,
            key_info: false,
        },
        redactor,
        log: &Silent,
        input_callbacks: &[],
        sentry: None,
        logger_fn: None,
        now: DateTime::parse_from_rfc3339("2026-01-01T00:05:00+00:00").unwrap(),
    };
    (state, env)
}

fn message(text: &str) -> ChatMessage {
    ChatMessage {
        role: Role::User,
        content: Some(MessageContent::Text(text.to_owned())),
        name: None,
        tool_calls: None,
        tool_call_id: None,
        passthrough: IndexMap::new(),
    }
}

fn body_with(optional: Object) -> Object {
    let messages = [message("hi")];
    let environment = BTreeMap::new();
    let downloads = RefuseDownloads;
    let input = TransformInput {
        model: "gpt-4o",
        messages: &messages,
        optional_params: &optional,
        environment: &environment,
        downloads: &downloads,
    };
    transform_request(&input, Mode::Sync).unwrap()
}

fn stream_options(body: &Object) -> &Object {
    let Some(Json::Object(options)) = body.get("stream_options") else {
        panic!("stream_options is an object: {body:?}");
    };
    options
}

fn stream(flag: bool) -> Object {
    Object::from_iter([(String::from("stream"), Json::Bool(flag))])
}

#[test]
fn transform_request_adds_include_usage_to_a_stream_without_options() {
    let body = body_with(stream(true));
    assert_eq!(body.get("model"), Some(&Json::Text(String::from("gpt-4o"))));
    assert_eq!(
        stream_options(&body),
        &Object::from_iter([(String::from("include_usage"), Json::Bool(true))])
    );
}

#[test]
fn transform_request_keeps_existing_stream_options_on_a_stream() {
    let mut optional = stream(true);
    optional.insert(
        String::from("stream_options"),
        Json::Object(Object::from_iter([
            (String::from("include_usage"), Json::Bool(false)),
            (
                String::from("chunk_size"),
                Json::Number(Number::from(5_u64)),
            ),
        ])),
    );
    let body = body_with(optional);
    assert_eq!(
        stream_options(&body),
        &Object::from_iter([
            (String::from("include_usage"), Json::Bool(true)),
            (
                String::from("chunk_size"),
                Json::Number(Number::from(5_u64))
            ),
        ])
    );
}

#[test]
fn transform_request_treats_a_null_stream_options_as_empty() {
    let mut optional = stream(true);
    optional.insert(String::from("stream_options"), Json::Null);
    let body = body_with(optional);
    assert_eq!(
        stream_options(&body),
        &Object::from_iter([(String::from("include_usage"), Json::Bool(true))])
    );
}

#[test]
fn transform_request_without_a_stream_leaves_stream_options_alone() {
    let mut optional = stream(false);
    optional.insert(
        String::from("stream_options"),
        Json::Object(Object::from_iter([(
            String::from("include_usage"),
            Json::Bool(false),
        )])),
    );
    let body = body_with(optional);
    assert_eq!(
        stream_options(&body),
        &Object::from_iter([(String::from("include_usage"), Json::Bool(false))])
    );
}

#[test]
fn transform_request_without_options_or_stream_adds_no_stream_options() {
    let body = body_with(Object::new());
    assert_eq!(body.get("stream_options"), None);
}

#[rstest]
#[case::text(Json::Text(String::from("chunked")))]
#[case::number(Json::Number(Number::from(5_u64)))]
#[case::list(Json::List(Vec::new()))]
#[case::boolean(Json::Bool(false))]
fn transform_request_with_a_non_mapping_stream_options_is_a_400(#[case] options: Json) {
    let mut optional = stream(true);
    optional.insert(String::from("stream_options"), options);
    let messages = [message("hi")];
    let environment = BTreeMap::new();
    let downloads = RefuseDownloads;
    let input = TransformInput {
        model: "gpt-4o",
        messages: &messages,
        optional_params: &optional,
        environment: &environment,
        downloads: &downloads,
    };
    let error = transform_request(&input, Mode::Sync).unwrap_err();
    assert!(matches!(&error, EdenAiChatError::StreamOptions));
    assert_eq!(error.status_code(), 400);
    assert_eq!(error.to_string(), "stream_options must be a JSON object");
}

#[derive(Debug, Clone, Copy)]
enum Answer {
    Yes,
    No,
    Failed,
}

#[derive(Debug, Error)]
#[error("no reasoning capability")]
struct CapabilityFailure;

struct Capability(Answer);

impl ReasoningLookup for Capability {
    type Error = CapabilityFailure;

    fn supports_reasoning(&self, _model: &str) -> Result<bool, Self::Error> {
        match self.0 {
            Answer::Yes => Ok(true),
            Answer::No => Ok(false),
            Answer::Failed => Err(CapabilityFailure),
        }
    }
}

fn catalog(models: &[&str]) -> CatalogCapabilities {
    CatalogCapabilities {
        chat_models: models.iter().map(|model| (*model).to_owned()).collect(),
        text_models: BTreeSet::new(),
    }
}

#[test]
fn supported_params_adds_reasoning_effort_when_the_model_reasons() {
    let params = supported_params(&Capability(Answer::Yes), "gpt-4o", &catalog(&["gpt-4o"]));
    assert_eq!(params.last(), Some(&"reasoning_effort"));
    assert!(params.contains(&"user"));
    assert!(params.contains(&"stream_options"));
}

#[test]
fn supported_params_omits_reasoning_effort_when_the_model_does_not_reason() {
    let params = supported_params(&Capability(Answer::No), "gpt-4o", &catalog(&["gpt-4o"]));
    assert!(!params.contains(&"reasoning_effort"));
    assert!(params.contains(&"user"));
}

#[test]
fn supported_params_omits_reasoning_effort_when_the_lookup_fails() {
    let params = supported_params(&Capability(Answer::Failed), "gpt-4o", &catalog(&["gpt-4o"]));
    assert!(!params.contains(&"reasoning_effort"));
}

#[test]
fn supported_params_keeps_the_parent_list_for_an_unlisted_model() {
    let params = supported_params(&Capability(Answer::Yes), "custom", &catalog(&[]));
    assert!(params.contains(&"response_format"));
    assert!(params.contains(&"stream"));
    assert!(!params.contains(&"user"));
    assert_eq!(params.last(), Some(&"reasoning_effort"));
}

fn key(text: &str) -> SecretString {
    SecretString::from(text.to_owned())
}

fn environment(api_key: Option<&str>, api_base: Option<&str>) -> EdenAiEnv {
    EdenAiEnv {
        api_key: api_key.map(key),
        api_base: api_base.map(str::to_owned),
        global_api_key: None,
    }
}

#[test]
fn get_api_key_reads_the_explicit_key_then_the_environment() {
    let environment = environment(Some("environment"), None);
    assert_eq!(
        get_api_key(Some(&key("explicit")), &environment),
        Some(key("explicit"))
    );
    assert_eq!(get_api_key(None, &environment), Some(key("environment")));
    assert_eq!(get_api_key(None, &environment(None, None)), None);
}

#[rstest]
#[case::default_base(None, None, EDENAI_API_BASE)]
#[case::explicit_base(Some("https://proxy.example/v3"), None, "https://proxy.example/v3")]
#[case::explicit_wins(
    Some("https://proxy.example/v3"),
    Some("https://gateway.example/v3"),
    "https://proxy.example/v3"
)]
#[case::environment_base(None, Some("https://gateway.example/v3"), "https://gateway.example/v3")]
fn get_api_base_resolves_the_explicit_then_the_environment_then_the_default(
    #[case] api_base: Option<&str>,
    #[case] environment_base: Option<&str>,
    #[case] expected: &str,
) {
    let environment = environment(None, environment_base);
    assert_eq!(get_api_base(api_base, &environment), expected);
}

fn log_input() -> LogInput {
    LogInput {
        messages: HiddenJson::List(vec![HiddenJson::Object(IndexMap::from([(
            String::from("role"),
            HiddenJson::Text(String::from("user")),
        )]))]),
        data: RawBody::Object(IndexMap::from([(
            String::from("model"),
            HiddenJson::Text(String::from("m")),
        )])),
    }
}

fn reply_options() -> ReplyOptions<'static> {
    ReplyOptions {
        json_mode: false,
        provider: Some("edenai"),
        base_model: Some("base"),
        identifier: Uuid::nil,
        clock: || 7,
    }
}

fn additional_headers(reply: &ChatReply) -> Value {
    serde_json::to_value(
        reply
            .hidden_params
            .get("additional_headers")
            .expect("the hidden params carry the reply headers"),
    )
    .unwrap()
}

#[test]
fn transform_response_lifts_the_cost_into_hidden_params() {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let (mut state, env) = fixture(&redactor);
    let headers = IndexMap::from([(String::from("x-request-id"), String::from("req-1"))]);
    let body = br#"{"id":"c","object":"chat.completion","created":1,"model":"m","choices":[],"usage":{"prompt_tokens":1,"completion_tokens":2,"total_tokens":3},"cost":0.25}"#;
    let reply = transform_response(ReplyRequest {
        body,
        headers: &headers,
        state: &mut state,
        env: &env,
        input: log_input(),
        options: reply_options(),
    })
    .unwrap();
    assert_eq!(reply.response.model, "edenai/m");
    assert_eq!(reply.hidden_model.as_deref(), Some("base"));
    let additional = additional_headers(&reply);
    assert_eq!(additional[RESPONSE_COST_HEADER], json!(0.25));
    assert_eq!(additional["llm_provider-x-request-id"], json!("req-1"));
}

#[test]
fn transform_response_without_a_cost_keeps_only_the_reply_headers() {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let (mut state, env) = fixture(&redactor);
    let headers = IndexMap::from([(String::from("x-request-id"), String::from("req-1"))]);
    let body = br#"{"id":"c","object":"chat.completion","created":1,"model":"m","choices":[]}"#;
    let reply = transform_response(ReplyRequest {
        body,
        headers: &headers,
        state: &mut state,
        env: &env,
        input: log_input(),
        options: reply_options(),
    })
    .unwrap();
    let additional = additional_headers(&reply);
    assert_eq!(additional[RESPONSE_COST_HEADER], Value::Null);
    assert_eq!(additional["llm_provider-x-request-id"], json!("req-1"));
}

#[test]
fn chunk_parser_puts_the_cost_on_a_usage_the_chunk_carries() {
    let chunk = json!({
        "id": "c",
        "object": "chat.completion.chunk",
        "created": 1,
        "model": "m",
        "choices": [],
        "usage": {"prompt_tokens": 1, "completion_tokens": 2, "total_tokens": 3},
        "cost": 0.5
    });
    let parsed = chunk_parser(&chunk, Uuid::nil, || 7).unwrap();
    let Some(Value::Object(usage)) = parsed.usage else {
        panic!("the chunk carries its usage");
    };
    assert_eq!(usage.get("cost"), Some(&json!(0.5)));
    assert_eq!(usage.get("total_tokens"), Some(&json!(3)));
}

#[test]
fn chunk_parser_reads_the_cost_through_the_header_alias() {
    let chunk = json!({
        "id": "c",
        "object": "chat.completion.chunk",
        "created": 1,
        "model": "m",
        "choices": [],
        "usage": {"prompt_tokens": 1, "completion_tokens": 2, "total_tokens": 3},
        "x-edenai-cost": "2"
    });
    let parsed = chunk_parser(&chunk, Uuid::nil, || 7).unwrap();
    let Some(Value::Object(usage)) = parsed.usage else {
        panic!("the chunk carries its usage");
    };
    assert_eq!(usage.get("cost"), Some(&json!(2)));
}

#[test]
fn chunk_parser_drops_a_cost_when_the_chunk_has_no_usage() {
    let chunk = json!({
        "id": "c",
        "object": "chat.completion.chunk",
        "created": 1,
        "model": "m",
        "choices": [],
        "cost": 0.5
    });
    let parsed = chunk_parser(&chunk, Uuid::nil, || 7).unwrap();
    assert_eq!(parsed.usage, None);
}

#[test]
fn chunk_parser_without_a_cost_leaves_the_usage_alone() {
    let chunk = json!({
        "id": "c",
        "object": "chat.completion.chunk",
        "created": 1,
        "model": "m",
        "choices": [],
        "usage": {"prompt_tokens": 1, "completion_tokens": 2, "total_tokens": 3}
    });
    let parsed = chunk_parser(&chunk, Uuid::nil, || 7).unwrap();
    let Some(Value::Object(usage)) = parsed.usage else {
        panic!("the chunk carries its usage");
    };
    assert_eq!(usage.get("cost"), None);
}

#[test]
fn chunk_parser_rejects_a_chunk_that_is_not_an_object() {
    let error = chunk_parser(&json!("chunk"), Uuid::nil, || 7).unwrap_err();
    assert!(matches!(error, StreamError::Chunk(_)));
}

#[test]
fn chunk_parser_reports_an_error_chunk() {
    let chunk = json!({"error": {"message": "boom", "code": 429}});
    let error = chunk_parser(&chunk, Uuid::nil, || 7).unwrap_err();
    let StreamError::Provider { message, status } = error else {
        panic!("the chunk carries a provider error");
    };
    assert_eq!(message, "boom");
    assert_eq!(status, 429);
}

fn reply_headers() -> Headers {
    Headers::from([(String::from("x-request-id"), String::from("req-1"))])
}

#[test]
fn get_models_prefixes_every_catalog_id() {
    let headers = reply_headers();
    let ids = get_models(&CatalogReply {
        status: 200,
        headers: &headers,
        text: r#"{"data":[{"id":"a"},{"id":"b"}]}"#,
    })
    .unwrap();
    assert_eq!(ids, ["edenai/a", "edenai/b"]);
}

#[test]
fn get_models_returns_an_empty_catalog_for_an_empty_data_list() {
    let headers = reply_headers();
    let ids = get_models(&CatalogReply {
        status: 200,
        headers: &headers,
        text: r#"{"data":[]}"#,
    })
    .unwrap();
    assert!(ids.is_empty());
}

#[test]
fn get_models_keeps_the_status_text_and_headers_of_a_failed_reply() {
    let headers = reply_headers();
    let error = get_models(&CatalogReply {
        status: 404,
        headers: &headers,
        text: "not found",
    })
    .unwrap_err();
    assert_eq!(error.status_code, 404);
    assert_eq!(error.message, "not found");
    assert_eq!(error.headers, headers);
}

#[test]
fn get_models_turns_a_bad_catalog_body_into_a_500_naming_the_field() {
    let headers = reply_headers();
    let error = get_models(&CatalogReply {
        status: 200,
        headers: &headers,
        text: "{}",
    })
    .unwrap_err();
    assert_eq!(error.status_code, 500);
    assert!(error.message.contains("data"), "{}", error.message);
    assert_eq!(error.headers, headers);
}

#[rstest]
#[case::default_base(None, None, "https://api.edenai.run/v3/models")]
#[case::explicit_base(
    Some("https://proxy.example/v3"),
    None,
    "https://proxy.example/v3/models"
)]
#[case::environment_base(
    None,
    Some("https://gateway.example/v3"),
    "https://gateway.example/v3/models"
)]
#[case::trailing_slash(
    Some("https://proxy.example/v3/"),
    None,
    "https://proxy.example/v3//models"
)]
fn models_url_is_the_resolved_base_then_models(
    #[case] api_base: Option<&str>,
    #[case] environment_base: Option<&str>,
    #[case] expected: &str,
) {
    let environment = environment(None, environment_base);
    assert_eq!(models_url(api_base, &environment), expected);
}
