#![cfg(test)]

use super::shared::{
    analyzer, anonymizer, chat_content, content_text, context, messages, request_data,
    response_body,
};
use crate::presidio_response::error::PresidioResponseError;
use crate::presidio_response::logging::logging_hook;

#[test]
fn logging_hook_runs_the_async_hook_when_no_loop_is_running() {
    let analyzer = analyzer("http://presidio.invalid", Some("MASKED"));
    let anonymizer = anonymizer("http://presidio.invalid");
    let context = context(&analyzer, &anonymizer);
    let mut request = request_data(r#"{"messages":[{"role":"user","content":"Alice"}]}"#);
    let mut response =
        response_body(r#"{"choices":[{"message":{"role":"assistant","content":"Bob"}}]}"#);

    logging_hook(context, &mut request, &mut response, "acompletion").expect("logging hook");

    assert_eq!(
        content_text(messages(&request)[0].content.as_ref()).as_deref(),
        Some("MASKED")
    );
    assert_eq!(chat_content(&response).as_deref(), Some("MASKED"));
}

#[tokio::test]
async fn logging_hook_runs_the_async_hook_from_a_running_loop() {
    let analyzer = analyzer("http://presidio.invalid", Some("MASKED"));
    let anonymizer = anonymizer("http://presidio.invalid");
    let context = context(&analyzer, &anonymizer);
    let mut request = request_data(r#"{"messages":[{"role":"user","content":"Alice"}]}"#);
    let mut response =
        response_body(r#"{"choices":[{"message":{"role":"assistant","content":"Bob"}}]}"#);

    logging_hook(context, &mut request, &mut response, "acompletion").expect("logging hook");

    assert_eq!(
        content_text(messages(&request)[0].content.as_ref()).as_deref(),
        Some("MASKED")
    );
    assert_eq!(chat_content(&response).as_deref(), Some("MASKED"));
}

#[test]
fn logging_hook_leaves_other_call_types_untouched() {
    let analyzer = analyzer("http://presidio.invalid", Some("MASKED"));
    let anonymizer = anonymizer("http://presidio.invalid");
    let context = context(&analyzer, &anonymizer);
    let mut request = request_data(r#"{"messages":[{"role":"user","content":"Alice"}]}"#);
    let mut response =
        response_body(r#"{"choices":[{"message":{"role":"assistant","content":"Bob"}}]}"#);

    logging_hook(context, &mut request, &mut response, "embedding").expect("logging hook");

    assert_eq!(
        content_text(messages(&request)[0].content.as_ref()).as_deref(),
        Some("Alice")
    );
    assert_eq!(chat_content(&response).as_deref(), Some("Bob"));
}

#[test]
fn logging_hook_returns_the_error_from_the_async_hook() {
    let analyzer = analyzer("http://presidio.invalid", Some("MASKED"));
    let anonymizer = anonymizer("http://presidio.invalid");
    let context = context(&analyzer, &anonymizer);
    let mut request = request_data(r#"{"messages":[{"content":"Alice"}],"metadata":5}"#);
    let mut response = response_body(r#"{"choices":[]}"#);

    let error = logging_hook(context, &mut request, &mut response, "acompletion")
        .expect_err("the malformed metadata fails the hook");

    assert!(matches!(error, PresidioResponseError::Actions(_)));
}
