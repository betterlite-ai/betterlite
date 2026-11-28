use std::time::Duration;

use betterlite_logging::logging_pre_post::context::LogContext;
use betterlite_logging::logging_pre_post::env::{
    DebugSettings, GlobalFlags, LogRecord, LogSink, PrePostEnv,
};
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::decisions::Json;
use betterlite_openai::openai_base_config::Headers;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::DateTime;
use indexmap::IndexMap;

use super::support;
use crate::openai_dispatch::call::ChatCall;
use crate::openai_dispatch::error::OpenAiProviderError;
use crate::openai_dispatch::fixtures::answer::FixtureAnswer;
use crate::openai_dispatch::fixtures::logger::FixtureLogger;
use crate::openai_dispatch::logging::{B249Logger, LoggedCall, OpenAiLogger as _};
use crate::operation::OperationError;

const ERROR_BODY: &str = r#"{"error":{"message":"the model is overloaded","type":"server_error","param":null,"code":"overloaded"}}"#;

fn call(headers: Option<Headers>, extra: Option<Headers>) -> ChatCall {
    ChatCall {
        model: String::from("gpt-4o"),
        request: support::chat_request(
            r#"{"model":"gpt-4o","messages":[{"role":"user","content":"hi"}]}"#,
        ),
        credentials: support::credentials(
            Some("https://explicit.test/v1"),
            Some("sk-fixture-failure"),
            None,
        ),
        headers,
        extra_headers: extra,
        timeout: None,
        metadata: None,
    }
}

fn caller_headers() -> Headers {
    Headers::from([(String::from("x-header"), String::from("h"))])
}

fn extra_headers() -> Headers {
    Headers::from([(String::from("x-other"), String::from("1"))])
}

// The normal branch logs the error string beside the caller map and rethrows the provider failure.
pub(super) async fn check_normal_branch() {
    let answer = FixtureAnswer::new(500, ERROR_BODY.as_bytes().to_vec());
    let mut logger = FixtureLogger::default();
    let Err(error) = support::dispatch_chat(
        &support::composition(),
        call(Some(caller_headers()), Some(extra_headers())),
        &answer,
        &mut logger,
    )
    .await
    else {
        panic!("the provider failure");
    };
    assert_eq!(logger.calls.len(), 1);
    let Json::Text(message) = &logger.calls[0].original_response else {
        panic!("the error string");
    };
    assert_eq!(message, &error.to_string());
    assert_eq!(
        support::logged_header(&logger.calls[0], "x-header"),
        Some("h")
    );
    // completion() merged the extra headers into the map main.py logs
    assert_eq!(
        support::logged_header(&logger.calls[0], "x-other"),
        Some("1")
    );
    let OpenAiProviderError::Operation(OperationError::Provider { response }) = error else {
        panic!("the provider's own error");
    };
    assert_eq!(response.status, 500);
    assert_eq!(response.body, ERROR_BODY.as_bytes());
}

// With no caller map the normal branch logs no headers at all.
pub(super) async fn check_without_caller_headers() {
    let answer = FixtureAnswer::new(500, ERROR_BODY.as_bytes().to_vec());
    let mut logger = FixtureLogger::default();
    let error = support::dispatch_chat(
        &support::composition(),
        call(None, None),
        &answer,
        &mut logger,
    )
    .await;
    assert!(error.is_err());
    assert_eq!(logger.calls.len(), 1);
    assert!(logger.calls[0].headers.is_none());
}

async fn experimental_failure(headers: Option<Headers>, extra: Option<Headers>) -> FixtureLogger {
    let mut composition = support::composition();
    composition.provider.experimental_handler = true;
    let answer = FixtureAnswer::new(500, ERROR_BODY.as_bytes().to_vec());
    let mut logger = FixtureLogger::default();
    let error =
        support::dispatch_chat(&composition, call(headers, extra), &answer, &mut logger).await;
    assert!(error.is_err());
    logger
}

// The experimental branch logs the map validate_environment wrote: the caller's map and the key, and its own content type only when the caller named none.
pub(super) async fn check_experimental() {
    let logger = experimental_failure(Some(caller_headers()), Some(extra_headers())).await;
    assert_eq!(logger.calls.len(), 1);
    assert_eq!(
        support::logged_header(&logger.calls[0], "Authorization"),
        Some("Bearer sk-fixture-failure")
    );
    assert_eq!(
        support::logged_header(&logger.calls[0], "Content-Type"),
        Some("application/json")
    );
    assert_eq!(
        support::logged_header(&logger.calls[0], "x-other"),
        Some("1")
    );
    let mut headers = caller_headers();
    headers.insert(String::from("Content-Type"), String::from("text/plain"));
    let content_type_logger = experimental_failure(Some(headers), None).await;
    assert_eq!(content_type_logger.calls.len(), 1);
    assert_eq!(
        support::logged_header(&content_type_logger.calls[0], "Content-Type"),
        Some("text/plain")
    );
    assert_eq!(
        support::logged_header(&content_type_logger.calls[0], "content-type"),
        None
    );
}

// A preparation failure happens before the try block, so it owns no post_call.
pub(super) async fn check_preparation_failure_has_no_log() {
    let mut call = call(Some(caller_headers()), Some(extra_headers()));
    call.credentials = support::credentials(Some("https://explicit.test/v1"), None, None);
    let answer = FixtureAnswer::new(200, support::CHAT_REPLY.as_bytes().to_vec());
    let mut logger = FixtureLogger::default();
    let Err(error) =
        support::dispatch_chat(&support::composition(), call, &answer, &mut logger).await
    else {
        panic!("the missing key");
    };
    assert!(matches!(
        error,
        OpenAiProviderError::Operation(OperationError::MissingApiKey)
    ));
    assert!(logger.calls.is_empty());
    assert!(answer.requests().is_empty());
}

// The failure log carries the messages the call sent beside the error text.
pub(super) async fn check_logged_input() {
    let answer = FixtureAnswer::new(500, ERROR_BODY.as_bytes().to_vec());
    let mut logger = FixtureLogger::default();
    let _error = support::dispatch_chat(
        &support::composition(),
        call(None, None),
        &answer,
        &mut logger,
    )
    .await;
    let Json::List(messages) = &logger.calls[0].input else {
        panic!("the logged messages");
    };
    assert_eq!(messages.len(), 1);
}

struct Discard;

impl LogSink for Discard {
    fn emit(&self, _record: LogRecord) {}
}

fn logging_state(redactor: &Redactor) -> LoggingState {
    let init = r#"{"model":"gpt-4o","call_type":"completion","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1"}"#;
    LoggingState::new(
        serde_json::from_str::<LoggingInit>(init).expect("a logging init"),
        CorrelationSetup {
            context: &mut CorrelationContext::default(),
            settings: CorrelationSettings::default(),
        },
        redactor,
        || String::from("trace"),
        Duration::default,
    )
    .expect("a logging state")
}

fn logging_env(redactor: &Redactor) -> PrePostEnv<'_> {
    PrePostEnv {
        debug: DebugSettings::default(),
        globals: GlobalFlags::default(),
        decision: Decision {
            content: false,
            key_info: false,
        },
        redactor,
        log: &Discard,
        input_callbacks: &[],
        sentry: None,
        logger_fn: None,
        now: DateTime::parse_from_rfc3339("2026-01-01T00:00:05+00:00").expect("a time"),
    }
}

fn logged_caller_headers() -> IndexMap<String, Json> {
    let mut headers = IndexMap::new();
    headers.insert(String::from("x-header"), Json::Text(String::from("h")));
    headers
}

// B249's post_call stores the response it was handed, the input and the headers alone.
pub(super) fn check_b249_logger() {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("a redactor");
    let mut state = logging_state(&redactor);
    let env = logging_env(&redactor);
    let mut logger = B249Logger {
        completion: LogContext {
            state: &mut state,
            env: &env,
        },
    };
    logger.post_call(LoggedCall {
        input: Json::List(Vec::new()),
        original_response: Json::Text(String::from("the model is overloaded")),
        headers: Some(logged_caller_headers()),
    });
    let details = &state.model_call_details;
    assert_eq!(
        details.get("original_response"),
        Some(&Json::Text(String::from("the model is overloaded")))
    );
    let mut args = IndexMap::new();
    args.insert(
        String::from("headers"),
        Json::Object(logged_caller_headers()),
    );
    assert_eq!(details.get("additional_args"), Some(&Json::Object(args)));
}

pub(super) async fn check_all() {
    check_normal_branch().await;
    check_without_caller_headers().await;
    check_experimental().await;
    check_preparation_failure_has_no_log().await;
    check_logged_input().await;
    check_b249_logger();
}
