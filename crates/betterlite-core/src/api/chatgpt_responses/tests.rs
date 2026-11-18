#![cfg(test)]

use std::future::{Future, ready};
use std::str::from_utf8;
use std::time::Duration;

use betterlite_config::sections::chatgpt_headers::Settings as HeaderSettings;
use betterlite_logging::logging_pre_post::env::{
    DebugSettings, GlobalFlags, LogRecord, LogSink, PrePostEnv,
};
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::decisions::Json;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_openai::api::responses_objects::ResponsesApiResponse;
use betterlite_openai::chatgpt::chat::Authenticator;
use betterlite_openai::chatgpt::headers::PlatformEnvironment;
use betterlite_openai::chatgpt::headers::session::{LitellmParamsInput, SessionIdentity};
use betterlite_openai::chatgpt::login::error::{Failure, LoginError};
use betterlite_openai::error::OpenAiError;
use betterlite_openai::openai_base_config::Headers;
use betterlite_openai::response_events::{RawResponse, ReplyBody, ResponsesReply};
use betterlite_openai::response_input::ResponsesInput;
use betterlite_openai::response_input::error::ResponseInputError;
use betterlite_prompts::arguments::json::{Json as Body, Object};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::DateTime;
use indexmap::IndexMap;
use rstest::rstest;
use secrecy::SecretString;

use super::recovery;
use super::{
    ChatGptResponsesError, EnvironmentRequest, PROVIDER, ResponsesConfig,
    SUPPORTS_NATIVE_WEBSOCKET, transform_reply,
};
use crate::api::api_boundaries::{ApiLimitError, ApiLimits};
use crate::api::responses_sse_recovery::ResponsesSseRecoveryError;

const NOW: i64 = 1_700_000_000;
const BASE: &str = "BASE INSTRUCTIONS";
const ENCRYPTED: &str = "reasoning.encrypted_content";
const AUTH_BASE: &str = "https://auth.test/codex";
const JSON_TYPE: &str = "application/json";
const SSE_TYPE: &str = "text/event-stream";

#[derive(Clone, Copy)]
enum Token {
    Good,
    Denied,
    DeviceExpired,
    RefreshDenied,
}

struct ScriptedAuth {
    base: &'static str,
    token: Token,
    account: Option<&'static str>,
}

impl Authenticator for ScriptedAuth {
    fn api_base(&self) -> &str {
        self.base
    }

    fn access_token(&mut self) -> impl Future<Output = Result<SecretString, LoginError>> + Send {
        let token = match self.token {
            Token::Good => Ok(SecretString::from(String::from("token-1"))),
            Token::Denied => Err(LoginError::AccessToken(Failure::Status(403))),
            Token::DeviceExpired => Err(LoginError::DeviceCode(Failure::Expired)),
            Token::RefreshDenied => Err(LoginError::Refresh(Failure::Status(429))),
        };
        ready(token)
    }

    fn account_id(&mut self) -> Option<String> {
        self.account.map(str::to_owned)
    }
}

struct FixedPlatform;

impl PlatformEnvironment for FixedPlatform {
    fn os_type(&self) -> String {
        String::from("Linux")
    }

    fn os_release(&self) -> String {
        String::from("6.1")
    }

    fn arch(&self) -> String {
        String::from("x86_64")
    }
}

struct FixedSession;

impl SessionIdentity for FixedSession {
    fn generate(&self) -> String {
        String::from("generated-session")
    }
}

fn settings() -> HeaderSettings {
    HeaderSettings {
        originator_override: None,
        user_agent_override: None,
        user_agent_suffix: None,
        default_instructions_override: Some(String::from(BASE)),
        terminal: String::new(),
    }
}

fn config(token: Token) -> ResponsesConfig<ScriptedAuth> {
    ResponsesConfig {
        authenticator: ScriptedAuth {
            base: AUTH_BASE,
            token,
            account: Some("acct-1"),
        },
        header_settings: settings(),
    }
}

async fn validate_with(
    config: &mut ResponsesConfig<ScriptedAuth>,
    params: &LitellmParamsInput,
) -> Result<Headers, ChatGptResponsesError> {
    config
        .validate_environment(EnvironmentRequest {
            headers: Headers::new(),
            params,
            model: "gpt-5",
            platform: &FixedPlatform,
            session: &FixedSession,
        })
        .await
}

fn request_options(text: &str) -> Object {
    serde_json::from_str(text).expect("the options are an object")
}

fn transform_request(options: &Object) -> Result<Object, ChatGptResponsesError> {
    config(Token::Good).transform_request(
        "gpt-5",
        &ResponsesInput::Text(String::from("hi")),
        options,
        None,
    )
}

struct Sink;

impl LogSink for Sink {
    fn emit(&self, _record: LogRecord) {}
}

struct Fixture {
    redactor: Redactor,
    sink: Sink,
    state: LoggingState,
}

impl Fixture {
    fn new() -> Self {
        let init: LoggingInit = serde_json::from_str(
            r#"{"model":"gpt-5","call_type":"responses","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#,
        )
        .expect("the logging fixture is valid");
        let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the redactor builds");
        let state = LoggingState::new(
            init,
            CorrelationSetup {
                context: &mut CorrelationContext::default(),
                settings: CorrelationSettings::default(),
            },
            &redactor,
            || String::from("generated"),
            Duration::default,
        )
        .expect("the logging state builds");
        Self {
            redactor,
            sink: Sink,
            state,
        }
    }

    fn transform(
        &mut self,
        status: u16,
        headers: &IndexMap<String, String>,
        body: &str,
        limits: &ApiLimits,
    ) -> Result<ResponsesReply, ChatGptResponsesError> {
        let sink: &dyn LogSink = &self.sink;
        let env = PrePostEnv {
            debug: DebugSettings::default(),
            globals: GlobalFlags::default(),
            decision: Decision {
                content: false,
                key_info: false,
            },
            redactor: &self.redactor,
            log: sink,
            input_callbacks: &[],
            sentry: None,
            logger_fn: None,
            now: DateTime::parse_from_rfc3339("2026-01-01T00:00:05+00:00")
                .expect("the clock parses"),
        };
        transform_reply(
            &RawResponse {
                status,
                headers,
                text: body,
            },
            NOW,
            &mut self.state,
            &env,
            limits,
        )
    }
}

fn headers(content_type: &str) -> IndexMap<String, String> {
    IndexMap::from([
        (String::from("content-type"), String::from(content_type)),
        (String::from("x-request-id"), String::from("req-1")),
        (
            String::from("x-ratelimit-remaining-requests"),
            String::from("9"),
        ),
    ])
}

fn event(payload: &str) -> String {
    format!("data: {payload}")
}

fn completed(response: &str) -> String {
    event(&format!(
        r#"{{"type":"response.completed","response":{response}}}"#
    ))
}

fn done() -> String {
    completed(r#"{"id":"resp_1","created_at":1700000000,"object":"response"}"#)
}

fn item(index: i64, id: &str) -> String {
    event(&format!(
        r#"{{"type":"response.output_item.done","output_index":{index},"item":{{"id":"{id}"}}}}"#
    ))
}

fn text(index: i64, body: &str) -> String {
    event(&format!(
        r#"{{"type":"response.output_text.done","output_index":{index},"text":"{body}"}}"#
    ))
}

fn transformed(
    status: u16,
    headers: &IndexMap<String, String>,
    lines: &[String],
) -> Result<ResponsesReply, ChatGptResponsesError> {
    Fixture::new().transform(status, headers, &lines.join("\n"), &ApiLimits::default())
}

fn expect_error(result: Result<ResponsesReply, ChatGptResponsesError>) -> ChatGptResponsesError {
    match result {
        Ok(_reply) => panic!("the reply fails"),
        Err(error) => error,
    }
}

fn typed(reply: &ResponsesReply) -> &ResponsesApiResponse {
    match &reply.response {
        ReplyBody::Typed(response) => response,
        ReplyBody::Lax(_) => panic!("the reply validates"),
    }
}

fn lax(reply: &ResponsesReply) -> &IndexMap<String, Json> {
    match &reply.response {
        ReplyBody::Lax(fields) => fields,
        ReplyBody::Typed(_) => panic!("the payload no record holds stays lax"),
    }
}

fn output_ids(response: &ResponsesApiResponse) -> Vec<String> {
    let value = serde_json::to_value(&response.output).expect("the output serializes");
    let items: Vec<Json> = serde_json::from_value(value).expect("the output is JSON");
    items
        .iter()
        .filter_map(|entry| match entry {
            Json::Object(fields) => match fields.get("id") {
                Some(Json::Text(id)) => Some(id.clone()),
                None | Some(_) => None,
            },
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => None,
        })
        .collect()
}

fn provider_error(error: ChatGptResponsesError) -> (u16, String) {
    let ChatGptResponsesError::Transform(OpenAiError::Provider(provider)) = error else {
        panic!("the provider failure passes through");
    };
    (provider.status_code, provider.message)
}

#[test]
fn provider_and_websocket_capability() {
    assert_eq!(PROVIDER, ProviderId::Chatgpt);
    assert!(!SUPPORTS_NATIVE_WEBSOCKET);
}

#[test]
fn url_prefers_the_explicit_base() {
    let config = config(Token::Good);
    assert_eq!(
        config.get_complete_url(Some("https://example.test/v1/")),
        "https://example.test/v1/responses"
    );
    assert_eq!(
        config.get_complete_url(Some("")),
        "https://auth.test/codex/responses"
    );
    assert_eq!(
        config.get_complete_url(None),
        "https://auth.test/codex/responses"
    );
}

#[test]
fn url_falls_back_to_the_default_base() {
    let config = ResponsesConfig {
        authenticator: ScriptedAuth {
            base: "",
            token: Token::Good,
            account: None,
        },
        header_settings: settings(),
    };
    assert_eq!(
        config.get_complete_url(None),
        "https://chatgpt.com/backend-api/codex/responses"
    );
}

#[tokio::test]
async fn environment_merges_caller_headers_over_defaults() {
    let mut config = config(Token::Good);
    let params = LitellmParamsInput::Mapping(
        serde_json::from_str(r#"{"session_id":"sess-1"}"#).expect("the params are a mapping"),
    );
    let mut headers = Headers::new();
    headers.insert(String::from("user-agent"), String::from("caller-agent"));
    headers.insert(String::from("x-custom"), String::from("1"));
    let merged = config
        .validate_environment(EnvironmentRequest {
            headers,
            params: &params,
            model: "gpt-5",
            platform: &FixedPlatform,
            session: &FixedSession,
        })
        .await
        .expect("the environment validates");
    assert_eq!(
        merged.get("Authorization").map(String::as_str),
        Some("Bearer token-1")
    );
    assert_eq!(
        merged.get("user-agent").map(String::as_str),
        Some("caller-agent")
    );
    assert_eq!(merged.get("session_id").map(String::as_str), Some("sess-1"));
    assert_eq!(
        merged.get("ChatGPT-Account-Id").map(String::as_str),
        Some("acct-1")
    );
    assert_eq!(
        merged.get("content-type").map(String::as_str),
        Some("application/json")
    );
    assert_eq!(
        merged.get("accept").map(String::as_str),
        Some("text/event-stream")
    );
    assert_eq!(
        merged.get("originator").map(String::as_str),
        Some("codex_cli_rs")
    );
    assert_eq!(merged.get("x-custom").map(String::as_str), Some("1"));
}

#[tokio::test]
async fn environment_generates_an_absent_session_id() {
    let mut config = config(Token::Good);
    let merged = validate_with(&mut config, &LitellmParamsInput::Absent)
        .await
        .expect("the environment validates");
    assert_eq!(
        merged.get("session_id").map(String::as_str),
        Some("generated-session")
    );
}

#[tokio::test]
async fn environment_omits_an_absent_account() {
    let mut config = ResponsesConfig {
        authenticator: ScriptedAuth {
            base: AUTH_BASE,
            token: Token::Good,
            account: None,
        },
        header_settings: settings(),
    };
    let merged = validate_with(&mut config, &LitellmParamsInput::Absent)
        .await
        .expect("the environment validates");
    assert!(!merged.contains_key("ChatGPT-Account-Id"));
}

#[tokio::test]
async fn environment_translates_an_access_token_failure() {
    let mut config = config(Token::Denied);
    let error = validate_with(&mut config, &LitellmParamsInput::Absent)
        .await
        .expect_err("the token is refused");
    let status = error.http_status();
    let message = error.to_string();
    assert!(matches!(
        error,
        ChatGptResponsesError::Authentication { ref model, .. } if model == "gpt-5"
    ));
    assert_eq!(status, Some(401));
    assert_eq!(message, "failed to obtain an access token");
}

#[tokio::test]
async fn environment_keeps_a_login_failure_status() {
    let mut device = config(Token::DeviceExpired);
    let device_error = validate_with(&mut device, &LitellmParamsInput::Absent)
        .await
        .expect_err("the device code expired");
    assert_eq!(device_error.http_status(), Some(408));
    assert!(matches!(device_error, ChatGptResponsesError::Login { .. }));
    let mut refresh = config(Token::RefreshDenied);
    let refresh_error = validate_with(&mut refresh, &LitellmParamsInput::Absent)
        .await
        .expect_err("the refresh failed");
    assert_eq!(refresh_error.http_status(), Some(429));
    assert!(matches!(refresh_error, ChatGptResponsesError::Login { .. }));
}

#[rstest]
#[case::absent("{}")]
#[case::null(r#"{"instructions":null}"#)]
fn request_inserts_the_default_instructions(#[case] options: &str) {
    let request = transform_request(&request_options(options)).expect("the request transforms");
    assert_eq!(
        request.get("instructions"),
        Some(&Body::Text(String::from(BASE)))
    );
    assert_eq!(request.get("model").and_then(Body::as_str), Some("gpt-5"));
    assert_eq!(request.get("input").and_then(Body::as_str), Some("hi"));
}

#[test]
fn request_keeps_instructions_that_contain_the_base() {
    let request = transform_request(&request_options(
        r#"{"instructions":"tail BASE INSTRUCTIONS head"}"#,
    ))
    .expect("valid");
    assert_eq!(
        request.get("instructions").and_then(Body::as_str),
        Some("tail BASE INSTRUCTIONS head")
    );
}

#[test]
fn request_prepends_the_base_to_other_instructions() {
    let request =
        transform_request(&request_options(r#"{"instructions":"custom"}"#)).expect("valid");
    assert_eq!(
        request.get("instructions").and_then(Body::as_str),
        Some("BASE INSTRUCTIONS\n\ncustom")
    );
}

#[test]
fn request_prepends_the_base_to_an_empty_instruction_string() {
    let request = transform_request(&request_options(r#"{"instructions":""}"#)).expect("valid");
    assert_eq!(
        request.get("instructions").and_then(Body::as_str),
        Some("BASE INSTRUCTIONS\n\n")
    );
}

#[rstest]
#[case::integer(r#"{"instructions":5}"#, "int")]
#[case::empty_list(r#"{"instructions":[]}"#, "list")]
fn request_rejects_instructions_that_are_not_text(#[case] options: &str, #[case] found: &str) {
    let error = transform_request(&request_options(options)).expect_err("the type is wrong");
    assert_eq!(error.http_status(), Some(400));
    assert!(matches!(
        error,
        ChatGptResponsesError::InstructionsNotText { found: actual } if actual == found
    ));
}

#[test]
fn request_forces_store_false_and_stream_true() {
    let request =
        transform_request(&request_options(r#"{"store":true,"stream":false}"#)).expect("valid");
    assert_eq!(request.get("store"), Some(&Body::Bool(false)));
    assert_eq!(request.get("stream"), Some(&Body::Bool(true)));
}

#[test]
fn request_appends_the_encrypted_content_include() {
    let request = transform_request(&request_options(r#"{"include":["a"]}"#))
        .expect("the request transforms");
    assert_eq!(
        request.get("include"),
        Some(&Body::List(vec![
            Body::Text(String::from("a")),
            Body::Text(String::from(ENCRYPTED))
        ]))
    );
}

#[test]
fn request_keeps_the_encrypted_content_include_once() {
    let request = transform_request(&request_options(
        r#"{"include":["reasoning.encrypted_content"]}"#,
    ))
    .expect("the request transforms");
    assert_eq!(
        request.get("include"),
        Some(&Body::List(vec![Body::Text(String::from(ENCRYPTED))]))
    );
}

#[test]
fn request_treats_a_null_include_as_empty() {
    let request =
        transform_request(&request_options(r#"{"include":null}"#)).expect("the request transforms");
    assert_eq!(
        request.get("include"),
        Some(&Body::List(vec![Body::Text(String::from(ENCRYPTED))]))
    );
}

#[test]
fn request_rejects_a_non_list_include() {
    let error = transform_request(&request_options(r#"{"include":"text"}"#))
        .expect_err("the type is wrong");
    assert_eq!(error.http_status(), Some(400));
    assert!(matches!(
        error,
        ChatGptResponsesError::IncludeNotList { found } if found == "str"
    ));
}

#[test]
fn request_prunes_unlisted_keys() {
    let request = transform_request(&request_options(
        r#"{"temperature":0.5,"max_output_tokens":16,"top_p":0.9}"#,
    ))
    .expect("the request transforms");
    assert_eq!(request.len(), 6);
    assert!(!request.contains_key("temperature"));
    assert!(!request.contains_key("max_output_tokens"));
    assert!(!request.contains_key("top_p"));
}

#[test]
fn request_rejects_a_duplicate_body_key() {
    let error = transform_request(&request_options(r#"{"model":"other"}"#))
        .expect_err("model is filled in");
    assert_eq!(error.http_status(), None);
    assert!(matches!(
        error,
        ChatGptResponsesError::Request(ResponseInputError::DuplicateBodyKey { key })
            if key == "model"
    ));
}

#[test]
fn request_propagates_input_failures() {
    let error =
        transform_request(&request_options(r#"{"tools":5}"#)).expect_err("the tools type is wrong");
    assert_eq!(error.http_status(), Some(400));
    assert!(matches!(
        error,
        ChatGptResponsesError::Request(ResponseInputError::ToolsNotAList { found })
            if found == "int"
    ));
}

#[test]
fn a_json_reply_is_decoded_by_the_models_transform() {
    let body = r#"{"id":"resp_1","created_at":1700000000,"model":"gpt-5","object":"response","status":"completed","output":[]}"#;
    let reply = transformed(200, &headers(JSON_TYPE), &[String::from(body)])
        .expect("the JSON reply decodes");
    let response = typed(&reply);
    assert_eq!(response.id, "resp_1");
    assert_eq!(response.created_at, NOW);
    assert_eq!(reply.headers, headers(JSON_TYPE));
    assert_eq!(
        reply
            .additional_headers
            .get("x-ratelimit-remaining-requests"),
        Some(&Json::Text(String::from("9")))
    );
    assert_eq!(
        reply.additional_headers.get("llm_provider-x-request-id"),
        Some(&Json::Text(String::from("req-1")))
    );
    assert_eq!(
        reply.additional_headers.get("llm_provider-content-type"),
        Some(&Json::Text(String::from("application/json")))
    );
}

#[test]
fn a_json_body_that_is_not_json_keeps_the_provider_status() {
    let error = expect_error(transformed(
        502,
        &headers(JSON_TYPE),
        &[String::from("nope")],
    ));
    assert_eq!(error.http_status(), Some(502));
    assert_eq!(provider_error(error), (502, String::from("nope")));
}

#[test]
fn a_json_body_without_created_at_keeps_the_provider_status() {
    let body = String::from(r#"{"id":"resp_1","output":[]}"#);
    let error = expect_error(transformed(400, &headers(JSON_TYPE), &[body.clone()]));
    assert_eq!(error.http_status(), Some(400));
    assert_eq!(provider_error(error), (400, body));
}

#[rstest]
#[case::upper_case_content_type(Some("TEXT/Event-Stream"), "", true)]
#[case::content_type_with_parameters(Some("text/event-stream; charset=utf-8"), "", true)]
#[case::json_content_type(Some("application/json"), "", false)]
#[case::leading_event_line(None, "event: response.completed", true)]
#[case::leading_data_line(None, "  data: {\"a\": 1}", true)]
#[case::embedded_event_line(None, "x\nevent: response.completed", true)]
#[case::embedded_data_line(None, "x\ndata: {}", true)]
#[case::inline_data_word(None, "x data: {}", false)]
#[case::plain_json(None, "{\"a\": 1}", false)]
fn sse_detection(#[case] content_type: Option<&str>, #[case] body: &str, #[case] expected: bool) {
    let mut sent = IndexMap::new();
    if let Some(value) = content_type {
        sent.insert(String::from("Content-Type"), String::from(value));
    }
    assert_eq!(recovery::should_parse_as_sse(&sent, body), expected);
}

#[test]
fn sse_recovers_text_only_output() {
    let body = vec![
        text(0, "hello"),
        completed(r#"{"id":"resp_1","created_at":1700000000,"object":"response","output":[]}"#),
    ];
    let reply = transformed(200, &headers(SSE_TYPE), &body).expect("the stream recovers");
    let response = typed(&reply);
    assert_eq!(response.id, "resp_1");
    assert_eq!(response.created_at, NOW);
    assert_eq!(response.output_text(), "hello");
    assert_eq!(output_ids(response), vec![String::from("msg_0")]);
}

#[test]
fn sse_keeps_a_nonempty_completed_output() {
    let body = vec![
        text(0, "ignored"),
        completed(
            r#"{"id":"resp_1","created_at":1700000000,"object":"response","output":[{"id":"done"}]}"#,
        ),
    ];
    let reply = transformed(200, &headers(SSE_TYPE), &body).expect("the stream recovers");
    let response = typed(&reply);
    assert_eq!(response.output_text(), "");
    assert_eq!(output_ids(response), vec![String::from("done")]);
}

#[test]
fn sse_keeps_an_output_that_is_not_a_list_as_sent() {
    let body = vec![
        text(0, "ignored"),
        completed(r#"{"id":"resp_1","created_at":1700000000,"object":"response","output":"kept"}"#),
    ];
    let reply = transformed(200, &headers(SSE_TYPE), &body).expect("the stream recovers");
    let fields = lax(&reply);
    assert_eq!(
        fields.get("output"),
        Some(&Json::Text(String::from("kept")))
    );
}

#[test]
fn sse_real_items_replace_text_at_the_same_index() {
    let body = vec![
        text(0, "synthetic"),
        item(1, "real-one"),
        item(0, "real-zero"),
        done(),
    ];
    let reply = transformed(200, &headers(SSE_TYPE), &body).expect("the stream recovers");
    let response = typed(&reply);
    assert_eq!(
        output_ids(response),
        vec![String::from("real-zero"), String::from("real-one")]
    );
    assert_eq!(response.output_text(), "");
}

#[test]
fn sse_keeps_text_at_an_index_without_a_real_item() {
    let body = vec![item(0, "real"), text(1, "kept"), done()];
    let reply = transformed(200, &headers(SSE_TYPE), &body).expect("the stream recovers");
    let response = typed(&reply);
    assert_eq!(response.output_text(), "kept");
    assert_eq!(
        output_ids(response),
        vec![String::from("real"), String::from("msg_1")]
    );
}

#[test]
fn sse_stops_at_the_first_completed_event() {
    let body = vec![
        completed(
            r#"{"id":"resp_first","created_at":1700000000,"object":"response","output":[{"id":"first"}]}"#,
        ),
        completed(
            r#"{"id":"resp_second","created_at":1700000000,"object":"response","output":[{"id":"second"}]}"#,
        ),
        item(0, "later"),
    ];
    let reply = transformed(200, &headers(SSE_TYPE), &body).expect("the stream recovers");
    let response = typed(&reply);
    assert_eq!(response.id, "resp_first");
    assert_eq!(output_ids(response), vec![String::from("first")]);
}

#[test]
fn sse_stops_at_a_completed_event_with_a_malformed_response() {
    let body = vec![
        event(r#"{"type":"response.completed","response":[]}"#),
        completed(r#"{"id":"resp_later","created_at":1700000000,"object":"response"}"#),
    ];
    let error = expect_error(transformed(200, &headers(SSE_TYPE), &body));
    assert_eq!(error.http_status(), Some(200));
    let (status, message) = provider_error(error);
    assert_eq!(status, 200);
    assert_eq!(message, body.join("\n"));
}

#[test]
fn sse_a_failure_before_a_completion_is_not_an_error() {
    let body = vec![
        event(r#"{"type":"response.failed","error":{"message":"transient"}}"#),
        text(0, "recovered"),
        done(),
    ];
    let reply = transformed(200, &headers(SSE_TYPE), &body).expect("the stream recovers");
    assert_eq!(typed(&reply).output_text(), "recovered");
}

#[test]
fn sse_reports_the_most_recent_failure_message() {
    let body = vec![
        event(r#"{"type":"response.failed","error":{"message":"first"}}"#),
        event(r#"{"type":"error","error":"second"}"#),
    ];
    let error = expect_error(transformed(500, &headers(SSE_TYPE), &body));
    assert_eq!(error.http_status(), Some(500));
    assert_eq!(provider_error(error), (500, String::from("second")));
}

#[test]
fn sse_reads_a_failure_from_the_response_field() {
    let body = vec![event(
        r#"{"type":"response.failed","response":{"error":{"code":"x"}}}"#,
    )];
    let error = expect_error(transformed(502, &headers(SSE_TYPE), &body));
    assert_eq!(provider_error(error), (502, String::from("{'code': 'x'}")));
}

#[test]
fn sse_falls_back_to_the_response_error_when_the_event_error_is_null() {
    let body = vec![event(
        r#"{"type":"response.failed","error":null,"response":{"error":"fallback"}}"#,
    )];
    let error = expect_error(transformed(500, &headers(SSE_TYPE), &body));
    assert_eq!(provider_error(error), (500, String::from("fallback")));
}

#[rstest]
#[case::integer("1700000005", 1_700_000_005)]
#[case::text("1700000007", 1_700_000_007)]
#[case::truncated_float("1700000006.9", 1_700_000_006)]
#[case::null("null", NOW)]
#[case::not_a_number("\"soon\"", NOW)]
#[case::boolean("true", NOW)]
fn sse_converts_created_at(#[case] created_at: &str, #[case] expected: i64) {
    let response =
        format!(r#"{{"id":"resp_1","created_at":{created_at},"object":"response","output":[]}}"#);
    let body = vec![completed(&response)];
    let reply = transformed(200, &headers(SSE_TYPE), &body).expect("the stream recovers");
    assert_eq!(typed(&reply).created_at, expected);
}

#[test]
fn sse_rejects_a_created_at_that_cannot_be_converted() {
    let body = vec![completed(
        r#"{"id":"resp_1","created_at":"inf","object":"response","output":[]}"#,
    )];
    let error = expect_error(transformed(200, &headers(SSE_TYPE), &body));
    assert!(matches!(error, ChatGptResponsesError::CreatedAt(_)));
    assert_eq!(error.http_status(), None);
}

#[test]
fn sse_keeps_a_payload_no_record_holds() {
    let body = vec![completed(r#"{"created_at":5,"object":"response"}"#)];
    let reply = transformed(200, &headers(SSE_TYPE), &body).expect("the stream recovers");
    let fields = lax(&reply);
    assert_eq!(fields.get("created_at"), Some(&Json::Number(5.into())));
    assert_eq!(
        fields.get("object"),
        Some(&Json::Text(String::from("response")))
    );
    assert!(!fields.contains_key("output"));
}

#[test]
fn sse_fills_the_output_of_a_payload_no_record_holds() {
    let body = vec![
        text(0, "recovered"),
        completed(r#"{"created_at":5,"object":"response"}"#),
    ];
    let reply = transformed(200, &headers(SSE_TYPE), &body).expect("the stream recovers");
    let fields = lax(&reply);
    let Some(Json::List(output)) = fields.get("output") else {
        panic!("the recovered output fills the payload");
    };
    assert_eq!(output.len(), 1);
}

#[test]
fn sse_attaches_raw_and_processed_headers() {
    let body = vec![done()];
    let reply = transformed(200, &headers(SSE_TYPE), &body).expect("the stream recovers");
    assert_eq!(reply.headers, headers(SSE_TYPE));
    assert_eq!(reply.additional_headers.get("x-request-id"), None);
    assert_eq!(
        reply.additional_headers.get("llm_provider-x-request-id"),
        Some(&Json::Text(String::from("req-1")))
    );
    assert_eq!(
        reply
            .additional_headers
            .get("x-ratelimit-remaining-requests"),
        Some(&Json::Text(String::from("9")))
    );
}

#[test]
fn sse_records_the_post_call_point() {
    let mut fixture = Fixture::new();
    let body = vec![done()].join("\n");
    fixture
        .transform(200, &headers(SSE_TYPE), &body, &ApiLimits::default())
        .expect("the stream recovers");
    let details = &fixture.state.model_call_details;
    assert!(matches!(
        details.get("original_response"),
        Some(Json::Text(recorded)) if *recorded == body
    ));
    assert_eq!(details.get("input"), Some(&Json::Null));
    assert_eq!(
        details.get("additional_args"),
        Some(&Json::Object(IndexMap::from([(
            String::from("complete_input_dict"),
            Json::Object(IndexMap::new())
        )])))
    );
    assert_eq!(
        details.get("log_event_type"),
        Some(&Json::Text(String::from("post_api_call")))
    );
}

#[test]
fn sse_marker_lines_do_not_complete_the_response() {
    let body = vec![
        String::from("event: response.completed"),
        String::from("data: [DONE]"),
    ];
    let error = expect_error(transformed(200, &headers(SSE_TYPE), &body));
    assert_eq!(provider_error(error).1, body.join("\n"));
}

#[test]
fn sse_handles_carriage_return_line_ends() {
    let body = [text(0, "hello"), done()].join("\r\n");
    let reply = Fixture::new()
        .transform(200, &headers(SSE_TYPE), &body, &ApiLimits::default())
        .expect("the stream recovers");
    assert_eq!(typed(&reply).output_text(), "hello");
}

fn limits(max_event_bytes: usize, max_items: usize) -> ApiLimits {
    ApiLimits {
        max_json_bytes: 16 * 1024 * 1024,
        max_event_bytes,
        max_items,
    }
}

#[test]
fn sse_rejects_an_event_over_the_boundary() {
    let body = vec![done()];
    let error = expect_error(Fixture::new().transform(
        200,
        &headers(SSE_TYPE),
        &body.join("\n"),
        &limits(8, 64 * 1024),
    ));
    assert_eq!(error.http_status(), Some(502));
    assert!(matches!(
        error,
        ChatGptResponsesError::Recovery(ResponsesSseRecoveryError::Limit(
            ApiLimitError::EventTooLarge { .. }
        ))
    ));
}

#[test]
fn sse_rejects_more_items_than_the_boundary() {
    let body = vec![item(0, "zero"), item(1, "one"), done()];
    let error = expect_error(Fixture::new().transform(
        200,
        &headers(SSE_TYPE),
        &body.join("\n"),
        &limits(1024 * 1024, 1),
    ));
    assert_eq!(error.http_status(), Some(400));
    assert!(matches!(
        error,
        ChatGptResponsesError::Recovery(ResponsesSseRecoveryError::Limit(
            ApiLimitError::TooManyItems { .. }
        ))
    ));
}

#[test]
fn recovery_errors_without_a_status_stay_statusless() {
    let bytes = from_utf8(b"\xff").expect_err("the bytes are not UTF-8");
    let utf8_error = ChatGptResponsesError::Recovery {
        source: ResponsesSseRecoveryError::Utf8(bytes),
    };
    assert_eq!(utf8_error.http_status(), None);
    let encode = ChatGptResponsesError::Encode {
        source: serde_json::from_str::<i32>("x").expect_err("the value is not an integer"),
    };
    assert_eq!(encode.http_status(), None);
    let generic = ChatGptResponsesError::Transform(OpenAiError::MissingKey { key: "x" });
    assert_eq!(generic.http_status(), None);
}
