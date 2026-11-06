#![cfg(test)]

use std::sync::Arc;
use std::sync::mpsc::sync_channel;
use std::thread;
use std::time::Duration;

use tokio::runtime::{Builder, Runtime};
use tokio::sync::Semaphore;
use tokio::time::timeout;

use super::shared::{
    NO_PARAMS, analyzer, anonymizer, chat_content, content_text, context, messages, request_data,
    response_body, spawn_mock,
};
use crate::presidio_actions::PresidioAnonymizer;
use crate::presidio_analysis::{PresidioAnalyzer, RequestOverrides};
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

const WATCHDOG: Duration = Duration::from_secs(10);
const HOOK_MESSAGES: &str = r#"{"messages":[{"role":"user","content":"Hook"}],"metadata":{}}"#;
const BOBBY_MESSAGES: &str = r#"{"messages":[{"role":"user","content":"Bobby"}]}"#;
const STREAM_BODY: &str = r#"{"object":"chat.completion.chunk","choices":[]}"#;
const BOBBY_CHAT: &str = r#"{"choices":[{"message":{"role":"assistant","content":"Bobby"}}]}"#;

type HookOutcome = (
    Result<(), PresidioResponseError>,
    Option<String>,
    Option<String>,
);

fn run_hook(
    analyzer: &PresidioAnalyzer,
    anonymizer: &PresidioAnonymizer,
    messages_json: &str,
    response_json: &str,
) -> HookOutcome {
    let context = context(analyzer, anonymizer);
    let mut request = request_data(messages_json);
    let mut response = response_body(response_json);
    let result = logging_hook(context, &mut request, &mut response, "acompletion");
    let message = messages(&request)
        .into_iter()
        .next()
        .and_then(|message| content_text(message.content.as_ref()));
    (result, message, chat_content(&response))
}

fn wait_for(runtime: &Runtime, signal: &Semaphore) {
    let waited = runtime.block_on(async { timeout(WATCHDOG, signal.acquire()).await });
    let _permit = waited
        .expect("the signal arrives within the watchdog timeout")
        .expect("the signal");
}

#[test]
fn a_hook_runtime_does_not_poison_the_shared_client_pool() {
    let runtime = Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("main runtime");
    let mock = runtime.block_on(spawn_mock(1));
    let shared_analyzer = Arc::new(analyzer(&mock.base, None));
    let anonymizer = Arc::new(anonymizer(&mock.base));
    let (sender, receiver) = sync_channel(1);

    thread::scope(|scope| {
        let hook_analyzer = Arc::clone(&shared_analyzer);
        let hook_anonymizer = Arc::clone(&anonymizer);
        let hook = scope.spawn(move || {
            let outcome = run_hook(&hook_analyzer, &hook_anonymizer, HOOK_MESSAGES, STREAM_BODY);
            sender.send(outcome).expect("the hook result");
        });
        wait_for(&runtime, &mock.anonymize_started);
        let slow_task = runtime.spawn({
            let analyzer = Arc::clone(&shared_analyzer);
            async move { analyzer.analyze_text("Slow", &OVERRIDES).await }
        });
        wait_for(&runtime, &mock.started);
        mock.anonymize_release.add_permits(1);
        let (result, _message, _reply) = receiver
            .recv_timeout(WATCHDOG)
            .expect("the hook returns within the watchdog timeout");
        result.expect("logging hook");
        hook.join().expect("the hook thread");
        mock.release.add_permits(1);
        let waited = runtime.block_on(async { timeout(WATCHDOG, slow_task).await });
        let slow = waited
            .expect("the slow request finishes within the watchdog timeout")
            .expect("the slow request task");
        let _slow = slow.expect("the slow request survives the hook runtime drop");
    });
}

#[test]
fn logging_hook_returns_from_a_current_thread_runtime() {
    let server = Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("server runtime");
    let mock = server.block_on(spawn_mock(1));
    let analyzer = analyzer(&mock.base, None);
    let anonymizer = anonymizer(&mock.base);
    let (sender, receiver) = sync_channel(1);

    thread::scope(|scope| {
        scope.spawn(move || {
            let runtime = Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("current-thread runtime");
            let outcome = runtime
                .block_on(async { run_hook(&analyzer, &anonymizer, BOBBY_MESSAGES, BOBBY_CHAT) });
            sender.send(outcome).expect("the hook result");
        });
        let (result, message, reply) = receiver
            .recv_timeout(WATCHDOG)
            .expect("the hook returns within the watchdog timeout");
        result.expect("logging hook");
        assert_eq!(message.as_deref(), Some("MASKED-Bobby"));
        assert_eq!(reply.as_deref(), Some("MASKED-Bobby"));
    });
}

#[test]
fn logging_hook_returns_from_a_single_worker_runtime() {
    let server = Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("server runtime");
    let mock = server.block_on(spawn_mock(1));
    let analyzer = Arc::new(analyzer(&mock.base, None));
    let anonymizer = Arc::new(anonymizer(&mock.base));
    let runner = Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .expect("runner runtime");
    let task = runner.spawn({
        let task_analyzer = Arc::clone(&analyzer);
        let task_anonymizer = Arc::clone(&anonymizer);
        async move { run_hook(&task_analyzer, &task_anonymizer, BOBBY_MESSAGES, BOBBY_CHAT) }
    });
    let (sender, receiver) = sync_channel(1);

    thread::scope(|scope| {
        scope.spawn(move || {
            let outcome = runner.block_on(task).expect("the hook task");
            sender.send(outcome).expect("the hook result");
        });
        let (result, message, reply) = receiver
            .recv_timeout(WATCHDOG)
            .expect("the hook returns within the watchdog timeout");
        result.expect("logging hook");
        assert_eq!(message.as_deref(), Some("MASKED-Bobby"));
        assert_eq!(reply.as_deref(), Some("MASKED-Bobby"));
    });
}
