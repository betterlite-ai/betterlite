use betterlite_models::decisions::Json;
use betterlite_openai::openai_base_config::Headers;

use super::support;
use crate::openai_dispatch::call::ChatCall;
use crate::openai_dispatch::error::OpenAiProviderError;
use crate::openai_dispatch::fixtures::answer::FixtureAnswer;
use crate::openai_dispatch::fixtures::logger::FixtureLogger;
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

// The experimental branch logs the map validate_environment wrote into: the caller map plus the key and content type.
pub(super) async fn check_experimental() {
    let mut composition = support::composition();
    composition.provider.experimental_handler = true;
    let answer = FixtureAnswer::new(500, ERROR_BODY.as_bytes().to_vec());
    let mut logger = FixtureLogger::default();
    let error = support::dispatch_chat(
        &composition,
        call(Some(caller_headers()), Some(extra_headers())),
        &answer,
        &mut logger,
    )
    .await;
    assert!(error.is_err());
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

pub(super) async fn check_all() {
    check_normal_branch().await;
    check_without_caller_headers().await;
    check_experimental().await;
    check_preparation_failure_has_no_log().await;
}
