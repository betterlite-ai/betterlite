#![cfg(test)]

use std::sync::Arc;
use std::time::Duration;

use tokio::runtime::Builder;
use tokio::time::timeout;

use super::shared::{
    NO_PARAMS, analyzer, anonymizer, chat_content, content_text, context, messages, request_data,
    response_body, spawn_mock,
};
use crate::presidio_analysis::RequestOverrides;
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

const OVERRIDES: RequestOverrides<'static> = RequestOverrides {
    language: None,
    dynamic_params: &NO_PARAMS,
};

#[test]
fn a_hook_runtime_does_not_poison_the_shared_client_pool() {
    let runtime = Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("main runtime");
    let mock = runtime.block_on(spawn_mock(1));
    let shared_analyzer = Arc::new(analyzer(&mock.base, None));
    let anonymizer = anonymizer(&mock.base);
    let in_flight = {
        let analyzer = Arc::clone(&shared_analyzer);
        runtime.spawn(async move { analyzer.analyze_text("Slow", &OVERRIDES).await })
    };
    let started = runtime.block_on(timeout(Duration::from_secs(5), mock.started.acquire()));
    let _started = started
        .expect("the slow request starts within the timeout")
        .expect("the slow request started");
    let context = context(&shared_analyzer, &anonymizer);
    let mut request =
        request_data(r#"{"messages":[{"role":"user","content":"Bobby"}],"metadata":{}}"#);
    let mut response = response_body(r#"{"object":"chat.completion.chunk","choices":[]}"#);

    logging_hook(context, &mut request, &mut response, "acompletion").expect("logging hook");

    let after = runtime.block_on(shared_analyzer.analyze_text("Carla", &OVERRIDES));
    let _shared_still_works = after.expect("the shared client works after the hook runtime drops");
    mock.release.add_permits(1);
    let _slow = runtime
        .block_on(in_flight)
        .expect("the slow request task")
        .expect("the slow analysis");
}
