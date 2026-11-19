#![cfg(test)]

use std::sync::Arc;

use betterlite_config::sections::settings::JsonValue;
use betterlite_openai::thread_runs::error::ThreadRunsError;
use betterlite_openai::thread_runs::records::RunRequest;
use http::{HeaderMap, HeaderName, HeaderValue};

use super::support::{self, CREATED, ERROR_BODY, Fixture, Recorder, run_request, status_of};
use crate::assistants::error::AssistantsRunError;
use crate::assistants::operations::RunDispatch;
use crate::azure_assistant_definitions::error::DefinitionsError;
use crate::credentials::source::CallableTokenProvider;

// Every SDK failure keeps its own kind, and none of them is retried or rewritten as text.
pub(super) async fn check() {
    rejects_a_bad_status().await;
    reports_transport_failures().await;
    reports_decode_failures().await;
    refuses_bad_path_ids().await;
    refuses_bad_run_ids().await;
    reports_poll_header_failures().await;
    reports_provider_failures().await;
    reports_stream_entry_failures().await;
}

// A status failure keeps its code and body whether the create or a later poll answers it.
async fn rejects_a_bad_status() {
    for (after_poll, failure, total) in [(false, 400_u16, 1_usize), (true, 500, 2)] {
        let recorder = Recorder::start();
        if after_poll {
            recorder.queue_poll(200, CREATED);
        }
        recorder.queue(failure, ERROR_BODY);
        let fixture = Fixture::new();
        let error = support::failed(&recorder, &fixture, &run_request()).await;
        let (status, body) = status_of(error);
        assert_eq!(status, failure);
        assert_eq!(body, ERROR_BODY.as_bytes());
        assert_eq!(recorder.requests().len(), total);
    }
}

async fn reports_transport_failures() {
    let recorder = Recorder::start();
    recorder.fail_next();
    let fixture = Fixture::new();
    let error = support::failed(&recorder, &fixture, &run_request()).await;
    assert!(matches!(
        error,
        AssistantsRunError::Runs(ThreadRunsError::Transport(_))
    ));
}

async fn decode_error(text: &str) -> AssistantsRunError {
    let recorder = Recorder::start();
    recorder.queue(200, text);
    let fixture = Fixture::new();
    support::failed(&recorder, &fixture, &run_request()).await
}

async fn reports_decode_failures() {
    let malformed = decode_error("not json").await;
    assert!(matches!(
        malformed,
        AssistantsRunError::Runs(ThreadRunsError::Malformed(_))
    ));
    let truncated = decode_error(r#"{"id":"run_1","#).await;
    assert!(matches!(
        truncated,
        AssistantsRunError::Runs(ThreadRunsError::Truncated(_))
    ));
    let empty = decode_error("").await;
    assert!(matches!(
        empty,
        AssistantsRunError::Runs(ThreadRunsError::Truncated(_))
    ));
    let shape = decode_error(r#"{"id":7}"#).await;
    assert!(matches!(
        shape,
        AssistantsRunError::Runs(ThreadRunsError::Shape(_))
    ));
}

fn request_with(thread: &str) -> RunRequest {
    let mut run = run_request();
    run.thread_id = thread.to_owned();
    run
}

async fn refuses_bad_path_ids() {
    let recorder = Recorder::start();
    let fixture = Fixture::new();
    let empty = support::failed(&recorder, &fixture, &request_with("")).await;
    assert!(matches!(
        empty,
        AssistantsRunError::Runs(ThreadRunsError::EmptyThreadId)
    ));
    let dot = support::failed(&recorder, &fixture, &request_with(".")).await;
    assert!(matches!(
        dot,
        AssistantsRunError::Runs(ThreadRunsError::DotSegment(_))
    ));
    let dots = support::failed(&recorder, &fixture, &request_with("..")).await;
    assert!(matches!(
        dots,
        AssistantsRunError::Runs(ThreadRunsError::DotSegment(_))
    ));
    assert!(recorder.requests().is_empty());
}

async fn refuses_bad_run_ids() {
    let fixture = Fixture::new();
    let recorder = Recorder::start();
    let mut created = support::parsed(CREATED);
    created["id"] = JsonValue::String(String::new());
    recorder.queue(200, &created.to_string());
    let empty = support::failed(&recorder, &fixture, &run_request()).await;
    assert!(matches!(
        empty,
        AssistantsRunError::Runs(ThreadRunsError::EmptyRunId)
    ));
    assert_eq!(recorder.requests().len(), 1);

    let second = Recorder::start();
    let mut dots_body = support::parsed(CREATED);
    dots_body["id"] = JsonValue::String(String::from(".."));
    second.queue(200, &dots_body.to_string());
    let dots = support::failed(&second, &fixture, &run_request()).await;
    assert!(matches!(
        dots,
        AssistantsRunError::Runs(ThreadRunsError::DotSegment(_))
    ));
    assert_eq!(second.requests().len(), 1);
}

async fn reports_poll_header_failures() {
    let fixture = Fixture::new();
    let mut moving = support::parsed(CREATED);
    moving["status"] = JsonValue::String(String::from("in_progress"));

    let recorder = Recorder::start();
    recorder.queue_poll(200, CREATED);
    let mut headers = HeaderMap::new();
    headers.insert(
        HeaderName::from_static("openai-poll-after-ms"),
        HeaderValue::from_static("soon"),
    );
    recorder.queue_with(200, headers, &moving.to_string());
    let text = support::failed(&recorder, &fixture, &run_request()).await;
    assert!(matches!(
        text,
        AssistantsRunError::Runs(ThreadRunsError::PollAfter(value)) if value == "soon"
    ));

    let raw_recorder = Recorder::start();
    raw_recorder.queue_poll(200, CREATED);
    let mut raw_headers = HeaderMap::new();
    raw_headers.insert(
        HeaderName::from_static("openai-poll-after-ms"),
        HeaderValue::from_bytes(b"\xff").unwrap(),
    );
    raw_recorder.queue_with(200, raw_headers, &moving.to_string());
    let bytes = support::failed(&raw_recorder, &fixture, &run_request()).await;
    assert!(matches!(
        bytes,
        AssistantsRunError::Runs(ThreadRunsError::PollAfter(_))
    ));
}

fn with_provider(token: Arc<dyn CallableTokenProvider>) -> Fixture {
    let mut fixture = Fixture::new();
    fixture.provider = Some(support::provider(token));
    fixture
}

async fn reports_provider_failures() {
    let recorder = Recorder::start();
    let failing = with_provider(Arc::new(support::FailingToken));
    let error = support::failed(&recorder, &failing, &run_request()).await;
    assert!(matches!(
        support::definitions(error),
        DefinitionsError::Provider { .. }
    ));
    let empty = with_provider(Arc::new(support::EmptyToken));
    let empty_error = support::failed(&recorder, &empty, &run_request()).await;
    assert!(matches!(
        support::definitions(empty_error),
        DefinitionsError::ProviderReply
    ));
    let nontext = with_provider(Arc::new(support::NonTextToken));
    let nontext_error = support::failed(&recorder, &nontext, &run_request()).await;
    assert!(matches!(
        support::definitions(nontext_error),
        DefinitionsError::ProviderReply
    ));
    assert!(recorder.requests().is_empty());
}

async fn reports_stream_entry_failures() {
    let fixture = Fixture::new();
    let dispatch = RunDispatch {
        arun_thread: None,
        stream: Some(true),
    };
    for status in [400_u16, 500] {
        let recorder = Recorder::start();
        recorder.queue(status, ERROR_BODY);
        let run = run_request();
        let manager = support::streamed(&recorder, &fixture, &run, dispatch, None).await;
        let error = manager.consume().await.unwrap_err();
        assert!(matches!(
            error,
            ThreadRunsError::Status(code, _) if code == status
        ));
    }
    let recorder = Recorder::start();
    recorder.fail_next();
    let run = run_request();
    let manager = support::streamed(&recorder, &fixture, &run, dispatch, None).await;
    let error = manager.consume().await.unwrap_err();
    assert!(matches!(error, ThreadRunsError::Transport(_)));
}
