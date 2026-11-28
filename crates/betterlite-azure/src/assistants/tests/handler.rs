#![cfg(test)]

use std::sync::Arc;
use std::time::Duration;

use betterlite_http::client_kind::ClientKind;
use betterlite_openai::thread_runs::stream::RunEventHandler;

use super::support::{self, Fixture, Recorder, RecordingHandler, STREAM_BODY, run_request};
use crate::assistants::operations::{RunCall, RunDispatch, RunInvocation};

fn streaming() -> RunDispatch {
    RunDispatch {
        arun_thread: None,
        stream: Some(true),
    }
}

// The SDK stream helpers return their manager and hand it the event handler they were given.
pub(super) async fn check() {
    passes_the_handler_through().await;
    omits_a_handler_left_out().await;
    delivers_events_before_ending().await;
    builds_streams_from_a_supplied_client().await;
}

async fn passes_the_handler_through() {
    let recorder = Recorder::start();
    let fixture = Fixture::new();
    let handler = RecordingHandler::start();
    let expected: Arc<dyn RunEventHandler> = Arc::<RecordingHandler>::clone(&handler);
    let manager = support::streamed(
        &recorder,
        &fixture,
        &run_request(),
        streaming(),
        Some(Arc::clone(&expected)),
    )
    .await;
    let attached = manager.handler.as_ref().unwrap();
    assert!(Arc::ptr_eq(attached, &expected));
    assert!(handler.calls().is_empty());
    assert!(recorder.requests().is_empty());
}

async fn omits_a_handler_left_out() {
    let recorder = Recorder::start();
    let fixture = Fixture::new();
    let manager = support::streamed(&recorder, &fixture, &run_request(), streaming(), None).await;
    assert!(manager.handler.is_none());
}

async fn delivers_events_before_ending() {
    let recorder = Recorder::start();
    recorder.queue(200, STREAM_BODY);
    let fixture = Fixture::new();
    let handler = RecordingHandler::start();
    let attached: Arc<dyn RunEventHandler> = Arc::<RecordingHandler>::clone(&handler);
    let manager = support::streamed(
        &recorder,
        &fixture,
        &run_request(),
        streaming(),
        Some(attached),
    )
    .await;
    assert!(handler.calls().is_empty());
    manager.consume().await.unwrap();
    assert_eq!(
        handler.calls(),
        ["thread.run.created", "thread.run.completed", "end"]
    );
}

async fn builds_streams_from_a_supplied_client() {
    let recorder = Recorder::start();
    recorder.queue(200, STREAM_BODY);
    let mut fixture = Fixture::new();
    fixture.params.timeout = Some(f64::NAN);
    let run = run_request();
    let supplied = support::supplied_client(ClientKind::Asynchronous, &recorder);
    let call = RunCall {
        dispatch: streaming(),
        run: &run,
        handler: None,
    };
    let invocation = support::api(&recorder)
        .run(&fixture.request(), Some(supplied), call)
        .await
        .unwrap();
    let RunInvocation::Streaming(manager) = invocation else {
        panic!("expected a stream");
    };
    assert_eq!(manager.kind, ClientKind::Asynchronous);
    assert_eq!(manager.read_timeout, Duration::from_secs(42));
    assert_eq!(
        manager.request.url.as_str(),
        "https://supplied.test/openai/threads/thread_1/runs?api-version=2024-01-01"
    );
}
