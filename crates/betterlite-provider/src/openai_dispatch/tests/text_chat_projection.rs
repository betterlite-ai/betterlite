use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::decisions::Json;
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

async fn dispatch(
    optional_params: &str,
    flags: Flags,
    source: FixtureTextSource,
) -> (OperationResult, FixtureLogger) {
    let mut logger = FixtureLogger::default();
    let result = support::dispatch_text(
        &support::composition(),
        call(optional_params, flags),
        &source,
        &mut logger,
    )
    .await
    .expect("the text dispatch");
    (result, logger)
}

async fn log_count(optional_params: &str, flags: Flags) -> usize {
    let source =
        FixtureTextSource::new(TextReply::new(200, support::TEXT_REPLY.as_bytes().to_vec()));
    dispatch(optional_params, flags, source).await.1.calls.len()
}

// main.py logs the text success for a truthy stream option or an async call, and not for a false or absent one.
pub(super) async fn check_stream_logging() {
    for (optional_params, flags, expected) in [
        ("{}", SYNC, 0),
        (r#"{"stream":false}"#, SYNC, 0),
        (r#"{"stream":null}"#, SYNC, 0),
        (r#"{"stream":true}"#, SYNC, 1),
        ("{}", ASYNC, 1),
        (r#"{"stream":true}"#, ASYNC, 1),
    ] {
        assert_eq!(log_count(optional_params, flags).await, expected);
    }
}

// A truthy stream keeps the text reply, a literally false one projects it to chat, and the text-only marker keeps it as text.
pub(super) async fn check_projection() {
    let stream_source =
        FixtureTextSource::new(TextReply::new(200, support::TEXT_REPLY.as_bytes().to_vec()));
    let (streamed, _stream_logger) = dispatch(r#"{"stream":true}"#, SYNC, stream_source).await;
    assert!(matches!(
        streamed,
        OperationResult::OpenAi(OpenAiResult::Text(_))
    ));
    let sync_source =
        FixtureTextSource::new(TextReply::new(200, support::TEXT_REPLY.as_bytes().to_vec()));
    let (sync, sync_logger) = dispatch("{}", SYNC, sync_source).await;
    assert!(matches!(
        sync,
        OperationResult::Chat(ChatCompletion::OpenAi(_))
    ));
    assert!(sync_logger.calls.is_empty());
    let text_only_source =
        FixtureTextSource::new(TextReply::new(200, support::TEXT_REPLY.as_bytes().to_vec()));
    let (text_only, text_only_logger) = dispatch("{}", TEXT_ONLY, text_only_source).await;
    assert!(matches!(
        text_only,
        OperationResult::OpenAi(OpenAiResult::Text(_))
    ));
    assert!(text_only_logger.calls.is_empty());
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

// A streamed text call keeps the opened stream and logs its absence beside the resolved headers.
pub(super) async fn check_text_stream_logging() {
    let source = FixtureTextSource::new(TextReply::streaming(
        200,
        vec![b"data: {\"id\":\"cmpl-1\"}\n\n".to_vec()],
    ));
    let mut logger = FixtureLogger::default();
    let result = support::dispatch_text(
        &support::composition(),
        call(r#"{"stream":true}"#, SYNC),
        &source,
        &mut logger,
    )
    .await
    .expect("the text stream");
    assert!(matches!(
        result,
        OperationResult::OpenAi(OpenAiResult::TextStream(_))
    ));
    assert_eq!(logger.calls.len(), 1);
    assert!(matches!(&logger.calls[0].original_response, Json::Null));
    assert!(logger.calls[0].headers.is_none());
}

pub(super) async fn check_all() {
    check_projection_predicate();
    check_stream_logging().await;
    check_projection().await;
    check_async_logging().await;
    check_text_stream_logging().await;
}
