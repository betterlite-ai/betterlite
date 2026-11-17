use betterlite_models::controls::json_value::JsonValue;
use betterlite_openai::openai_base_config::Headers;
use secrecy::SecretString;

use super::support;
use crate::openai_dispatch::OpenAiResult;
use crate::openai_dispatch::call::TextCall;
use crate::openai_dispatch::fixtures::logger::FixtureLogger;
use crate::openai_dispatch::fixtures::text_transport::{FixtureTextSource, TextReply};
use crate::openai_dispatch::text;
use crate::operation::{ChatCompletion, OperationResult};

// The two call markers main.py reads beside the stream option.
#[derive(Clone, Copy)]
struct Flags {
    acompletion: bool,
    text_completion: bool,
}

const SYNC: Flags = Flags {
    acompletion: false,
    text_completion: false,
};

const ASYNC: Flags = Flags {
    acompletion: true,
    text_completion: false,
};

const TEXT_ONLY: Flags = Flags {
    acompletion: false,
    text_completion: true,
};

fn call(optional_params: &str, flags: Flags) -> TextCall {
    TextCall {
        model: String::from("gpt-3.5-turbo-instruct"),
        messages: support::messages(r#"[{"role":"user","content":"hi"}]"#),
        optional_params: support::optional_params(optional_params),
        credentials: support::credentials(
            Some("https://explicit.test/v1"),
            Some("sk-explicit"),
            None,
        ),
        headers: None,
        timeout: None,
        acompletion: flags.acompletion,
        text_completion: flags.text_completion,
    }
}

fn plan(optional_params: &str, flags: Flags) -> text::TextPlan {
    text::plan(&support::composition(), &call(optional_params, flags)).expect("the text plan")
}

// main.py converts only when stream is literally False, and both flags are literally False.
pub(super) fn check_projection_predicate() {
    assert!(text::projects_to_chat(&plan("{}", SYNC)));
    assert!(text::projects_to_chat(&plan(r#"{"stream":false}"#, SYNC)));
    assert!(!text::projects_to_chat(&plan(r#"{"stream":true}"#, SYNC)));
    assert!(!text::projects_to_chat(&plan(r#"{"stream":null}"#, SYNC)));
    assert!(!text::projects_to_chat(&plan(r#"{"stream":0}"#, SYNC)));
    assert!(!text::projects_to_chat(&plan("{}", ASYNC)));
    assert!(!text::projects_to_chat(&plan("{}", TEXT_ONLY)));
}

async fn log_count(optional_params: &str, flags: Flags) -> usize {
    let source =
        FixtureTextSource::new(TextReply::new(200, support::TEXT_REPLY.as_bytes().to_vec()));
    let mut logger = FixtureLogger::default();
    support::dispatch_text(
        &support::composition(),
        call(optional_params, flags),
        &source,
        &mut logger,
    )
    .await
    .expect("the text dispatch");
    logger.calls.len()
}

// main.py logs the text success for a truthy stream option or an async call, and not for a false or absent one.
pub(super) async fn check_stream_logging() {
    assert_eq!(log_count("{}", SYNC).await, 0);
    assert_eq!(log_count(r#"{"stream":false}"#, SYNC).await, 0);
    assert_eq!(log_count(r#"{"stream":null}"#, SYNC).await, 0);
    assert_eq!(log_count(r#"{"stream":true}"#, SYNC).await, 1);
    assert_eq!(log_count("{}", ASYNC).await, 1);
    assert_eq!(log_count(r#"{"stream":true}"#, ASYNC).await, 1);
}

// A truthy stream keeps the text reply: only a literally false stream projects the reply to chat.
pub(super) async fn check_stream_keeps_text() {
    let source =
        FixtureTextSource::new(TextReply::new(200, support::TEXT_REPLY.as_bytes().to_vec()));
    let mut logger = FixtureLogger::default();
    let result = support::dispatch_text(
        &support::composition(),
        call(r#"{"stream":true}"#, SYNC),
        &source,
        &mut logger,
    )
    .await
    .expect("the text dispatch");
    assert!(matches!(
        result,
        OperationResult::OpenAi(OpenAiResult::Text(_))
    ));
}

// A synchronous text call becomes a chat completion and logs nothing.
pub(super) async fn check_sync_projection() {
    let source =
        FixtureTextSource::new(TextReply::new(200, support::TEXT_REPLY.as_bytes().to_vec()));
    let mut logger = FixtureLogger::default();
    let result = support::dispatch_text(
        &support::composition(),
        call("{}", SYNC),
        &source,
        &mut logger,
    )
    .await
    .expect("the text dispatch");
    assert!(matches!(
        result,
        OperationResult::Chat(ChatCompletion::OpenAi(_))
    ));
    assert!(logger.calls.is_empty());
}

// An async text call keeps the text reply and logs it with the resolved headers.
pub(super) async fn check_async_logging() {
    let mut composition = support::composition();
    composition.globals_headers = Some(Headers::from([(String::from("x-log"), String::from("1"))]));
    composition.globals.api_key = Some(SecretString::from("sk-global"));
    let source =
        FixtureTextSource::new(TextReply::new(200, support::TEXT_REPLY.as_bytes().to_vec()));
    let mut logger = FixtureLogger::default();
    let result = support::dispatch_text(
        &composition,
        call(r#"{"stream":false}"#, ASYNC),
        &source,
        &mut logger,
    )
    .await
    .expect("the text dispatch");
    let OperationResult::OpenAi(OpenAiResult::Text(reply)) = result else {
        panic!("the text reply itself");
    };
    let JsonValue::Object(fields) = reply.as_ref() else {
        panic!("a text reply object");
    };
    assert_eq!(
        fields.get("object"),
        Some(&JsonValue::Text(String::from("text_completion")))
    );
    assert_eq!(logger.calls.len(), 1);
    assert_eq!(support::logged_header(&logger.calls[0], "x-log"), Some("1"));
}

// The text-only marker keeps the reply as text instead of projecting it, and main.py logs nothing.
pub(super) async fn check_text_completion_marker() {
    let source =
        FixtureTextSource::new(TextReply::new(200, support::TEXT_REPLY.as_bytes().to_vec()));
    let mut logger = FixtureLogger::default();
    let result = support::dispatch_text(
        &support::composition(),
        call("{}", TEXT_ONLY),
        &source,
        &mut logger,
    )
    .await
    .expect("the text dispatch");
    assert!(matches!(
        result,
        OperationResult::OpenAi(OpenAiResult::Text(_))
    ));
    assert!(logger.calls.is_empty());
}

pub(super) async fn check_all() {
    check_projection_predicate();
    check_stream_logging().await;
    check_stream_keeps_text().await;
    check_sync_projection().await;
    check_async_logging().await;
    check_text_completion_marker().await;
}
