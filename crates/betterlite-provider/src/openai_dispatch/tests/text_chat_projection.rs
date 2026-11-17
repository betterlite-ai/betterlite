use betterlite_models::controls::json_value::JsonValue;
use betterlite_openai::openai_base_config::Headers;
use secrecy::SecretString;

use super::support;
use crate::openai_dispatch::OpenAiResult;
use crate::openai_dispatch::call::{Credentials, TextCall};
use crate::openai_dispatch::fixtures::{FixtureLogger, FixtureTextSource, TextReply};
use crate::openai_dispatch::text;
use crate::operation::{ChatCompletion, OperationResult};

fn call(optional_params: &str, acompletion: bool, text_completion: bool) -> TextCall {
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
        acompletion,
        text_completion,
    }
}

fn plan(optional_params: &str, acompletion: bool, text_completion: bool) -> text::TextPlan {
    text::plan(
        &support::composition(),
        &call(optional_params, acompletion, text_completion),
    )
    .expect("the text plan")
}

// main.py converts only when stream is literally False, and both flags are literally False.
pub fn check_projection_predicate() {
    assert!(text::projects_to_chat(&plan("{}", false, false)));
    assert!(text::projects_to_chat(&plan(
        r#"{"stream":false}"#,
        false,
        false
    )));
    assert!(!text::projects_to_chat(&plan(
        r#"{"stream":true}"#,
        false,
        false
    )));
    assert!(!text::projects_to_chat(&plan(
        r#"{"stream":null}"#,
        false,
        false
    )));
    assert!(!text::projects_to_chat(&plan(
        r#"{"stream":0}"#,
        false,
        false
    )));
    assert!(!text::projects_to_chat(&plan("{}", true, false)));
    assert!(!text::projects_to_chat(&plan("{}", false, true)));
}

// Text logs for a truthy stream or an async call; a text-only call logs nothing.
pub fn check_log_predicate() {
    assert!(!text::logs_post_call(&plan("{}", false, false)));
    assert!(!text::logs_post_call(&plan(
        r#"{"stream":false}"#,
        false,
        false
    )));
    assert!(text::logs_post_call(&plan(
        r#"{"stream":true}"#,
        false,
        false
    )));
    assert!(text::logs_post_call(&plan(
        r#"{"stream":"yes"}"#,
        false,
        false
    )));
    assert!(!text::logs_post_call(&plan(
        r#"{"stream":null}"#,
        false,
        false
    )));
    assert!(!text::logs_post_call(&plan(
        r#"{"stream":0}"#,
        false,
        false
    )));
    assert!(text::logs_post_call(&plan("{}", true, false)));
    assert!(!text::logs_post_call(&plan("{}", false, true)));
}

pub async fn check() {
    // a synchronous text call becomes a chat completion and logs nothing
    let text = FixtureTextSource::new(TextReply::new(200, support::TEXT_REPLY.as_bytes().to_vec()));
    let mut logger = FixtureLogger::default();
    let result = support::dispatch_text(
        &support::composition(),
        call("{}", false, false),
        &text,
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
pub async fn check_async_logging() {
    let mut composition = support::composition();
    composition.globals_headers = Some(Headers::from([(String::from("x-log"), String::from("1"))]));
    composition.globals.api_key = Some(SecretString::from("sk-global"));
    let text = FixtureTextSource::new(TextReply::new(200, support::TEXT_REPLY.as_bytes().to_vec()));
    let mut logger = FixtureLogger::default();
    let result = support::dispatch_text(
        &composition,
        call(r#"{"stream":false}"#, true, false),
        &text,
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
pub async fn check_text_completion_marker() {
    let text = FixtureTextSource::new(TextReply::new(200, support::TEXT_REPLY.as_bytes().to_vec()));
    let mut logger = FixtureLogger::default();
    let result = support::dispatch_text(
        &support::composition(),
        call("{}", false, true),
        &text,
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
