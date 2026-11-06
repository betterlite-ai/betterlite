#![cfg(test)]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use rstest::rstest;
use serde_json::Value;
use tokio::time::Instant;

use crate::assistants::client::ClientKind;
use crate::thread_runs::error::ThreadRunsError;
use crate::thread_runs::operations::{RunCall, RunDispatch, RunInvocation};
use crate::thread_runs::records::{RunEvent, RunReply, RunRequest, RunStatus};
use crate::thread_runs::stream::RunEventHandler;

use super::support::{
    COMPLETED, FAILED, GatedHandler, IN_PROGRESS, KEY, QUEUED, REQUIRES_ACTION, Recorder,
    RecordingHandler, Seen, harness, options, parse, wire,
};

const RUNS: &str = "https://example.test/v1/threads/thread_a/runs";
const RUN_1: &str = "https://example.test/v1/threads/thread_a/runs/run_1";
const REQUEST: &str = r#"{"thread_id":"thread_a","assistant_id":"asst_a","additional_instructions":"extra","instructions":"base","metadata":{"tag":"x"},"model":"gpt-4o-mini","tools":[{"type":"code_interpreter"}]}"#;
const POLL_BODY: &str = r#"{"assistant_id":"asst_a","additional_instructions":"extra","instructions":"base","metadata":{"tag":"x"},"model":"gpt-4o-mini","stream":false,"tools":[{"type":"code_interpreter"}]}"#;
const STREAM_BODY: &str = r#"{"assistant_id":"asst_a","additional_instructions":"extra","instructions":"base","metadata":{"tag":"x"},"model":"gpt-4o-mini","stream":true,"tools":[{"type":"code_interpreter"}]}"#;
const NULL_BODY: &str = r#"{"assistant_id":"asst_a","additional_instructions":null,"instructions":null,"metadata":null,"model":null,"stream":false,"tools":null}"#;
const NULL_STREAM_BODY: &str = r#"{"assistant_id":"asst_a","additional_instructions":null,"instructions":null,"metadata":null,"model":null,"stream":true,"tools":null}"#;
const ERROR_BODY: &str = r#"{"error":{"message":"no run","type":"invalid_request_error"}}"#;
const STEP: &str = r#"{"id":"step_1","object":"thread.run.step","created_at":1700000000,"run_id":"run_1","assistant_id":"asst_1","thread_id":"thread_a","type":"message_creation","status":"in_progress","step_details":{"type":"message_creation","message_creation":{"message_id":"msg_1"}}}"#;
const MESSAGE: &str = r#"{"id":"msg_1","object":"thread.message","created_at":1700000000,"thread_id":"thread_a","role":"assistant","content":[],"status":"in_progress"}"#;
const MESSAGE_DELTA: &str = r#"{"id":"msg_1","object":"thread.message.delta","delta":{}}"#;
const FUTURE: &str = r#"{"thing":true}"#;

#[derive(Clone, Copy)]
enum Facade {
    SyncRun,
    AsyncRun,
    SyncStream,
    AsyncStream,
}

fn request() -> RunRequest {
    serde_json::from_str(REQUEST).unwrap()
}

fn call(facade: Facade) -> RunCall<Recorder> {
    let dispatch = match facade {
        Facade::SyncRun => RunDispatch {
            arun_thread: false,
            stream: false,
        },
        Facade::AsyncRun => RunDispatch {
            arun_thread: true,
            stream: false,
        },
        Facade::SyncStream => RunDispatch {
            arun_thread: false,
            stream: true,
        },
        Facade::AsyncStream => RunDispatch {
            arun_thread: true,
            stream: true,
        },
    };
    RunCall {
        options: options(KEY),
        client: None,
        dispatch,
        handler: None,
    }
}

fn poll_wire() -> Seen {
    let mut seen = wire("GET", RUN_1, None);
    seen.poll_helper = Some(String::from("true"));
    seen
}

fn stream_wire(custom_handler: bool) -> Seen {
    let mut seen = wire("POST", RUNS, Some(STREAM_BODY));
    seen.stream_helper = Some(String::from("threads.runs.create_and_stream"));
    seen.custom_handler = Some(String::from(if custom_handler { "true" } else { "false" }));
    seen
}

fn frame(event: &str, data: &str) -> String {
    format!("event: {event}\ndata: {data}\n\n")
}

fn sse() -> String {
    format!(
        "{}{}{}data: [DONE]\n\n",
        frame("thread.run.created", QUEUED),
        frame("thread.run.in_progress", IN_PROGRESS),
        frame("thread.run.completed", COMPLETED),
    )
}

async fn poll_state(state: &str, arun_thread: bool) -> (Vec<Seen>, RunReply) {
    let (api, transport) = harness();
    transport.queue(200, &[], QUEUED);
    transport.queue(200, &[("openai-poll-after-ms", "0")], state);
    let facade = if arun_thread {
        Facade::AsyncRun
    } else {
        Facade::SyncRun
    };
    let outcome = api.run_thread(call(facade), &request()).await.unwrap();
    let RunInvocation::Polled { kind, run } = outcome else {
        panic!("expected a polled invocation");
    };
    let expected = if arun_thread {
        ClientKind::Async
    } else {
        ClientKind::Sync
    };
    assert_eq!(kind, expected);
    (transport.requests(), *run)
}

// Every field the provider sent survives, and a declared field it left out is a null in the reply.
fn assert_unchanged(reply: &RunReply, fixture: &str) {
    let output = serde_json::to_value(reply).unwrap();
    let Value::Object(expected) = parse(fixture) else {
        panic!("fixture is not an object");
    };
    for key in [
        "id",
        "object",
        "created_at",
        "status",
        "instructions",
        "model",
        "thread_id",
        "tools",
    ] {
        assert_eq!(output.get(key), expected.get(key), "{key}");
    }
    assert_eq!(output.get("completed_at"), expected.get("completed_at"));
    assert_eq!(output.get("cancelled_at"), Some(&Value::Null));
    assert_eq!(output.get("expires_at"), Some(&Value::Null));
}

// Both buffered paths hand back the created-and-polled run, whatever terminal state it reached.
async fn run_poll_completion() {
    for (state, arun_thread) in [
        (COMPLETED, false),
        (COMPLETED, true),
        (REQUIRES_ACTION, false),
        (FAILED, true),
    ] {
        let (seen, run) = poll_state(state, arun_thread).await;
        assert_unchanged(&run, state);
        assert_eq!(seen, vec![wire("POST", RUNS, Some(POLL_BODY)), poll_wire()]);
    }
    let (api, transport) = harness();
    transport.queue(404, &[], ERROR_BODY);
    let error = api
        .run_thread(call(Facade::SyncRun), &request())
        .await
        .err()
        .unwrap();
    assert!(matches!(error, ThreadRunsError::Status(404, _)), "{error}");
    assert_eq!(transport.requests().len(), 1);

    empty_thread_is_refused().await;
    dot_segments_are_refused().await;
    empty_run_id_is_refused().await;
    trailing_slash_base_keeps_one_slash().await;
    poll_intervals().await;
    poll_interval_failures().await;
    configured_timeout_is_the_deadline().await;
}

async fn empty_thread_is_refused() {
    let empty = RunRequest {
        thread_id: String::new(),
        ..request()
    };
    let (api, transport) = harness();
    let rejected = api
        .run_thread(call(Facade::SyncRun), &empty)
        .await
        .err()
        .unwrap();
    assert!(
        matches!(rejected, ThreadRunsError::EmptyThreadId),
        "{rejected}"
    );
    assert!(transport.requests().is_empty());
}

// Only the literal dot segments are refused; the SDK quotes first, so an encoded form reaches the provider.
async fn dot_segments_are_refused() {
    for thread_id in [".", ".."] {
        let dotted = RunRequest {
            thread_id: String::from(thread_id),
            ..request()
        };
        let (api, transport) = harness();
        let rejected = api
            .run_thread(call(Facade::SyncRun), &dotted)
            .await
            .err()
            .unwrap();
        assert!(
            matches!(rejected, ThreadRunsError::DotSegment(_)),
            "{thread_id}: {rejected}"
        );
        assert!(transport.requests().is_empty());
    }
    for (thread_id, url) in [
        ("%2e", "https://example.test/v1/threads/%252e/runs"),
        ("a[b]", "https://example.test/v1/threads/a%5Bb%5D/runs"),
    ] {
        let quoted = RunRequest {
            thread_id: String::from(thread_id),
            ..request()
        };
        let (api, transport) = harness();
        transport.queue(200, &[], QUEUED);
        transport.queue(200, &[("openai-poll-after-ms", "0")], COMPLETED);
        api.run_thread(call(Facade::SyncRun), &quoted)
            .await
            .unwrap();
        assert_eq!(transport.requests()[0].url, url);
    }
}

async fn empty_run_id_is_refused() {
    let created = QUEUED.replace(r#""run_1""#, r#""""#);
    let (api, transport) = harness();
    transport.queue(200, &[], &created);
    let rejected = api
        .run_thread(call(Facade::SyncRun), &request())
        .await
        .err()
        .unwrap();
    assert!(
        matches!(rejected, ThreadRunsError::EmptyRunId),
        "{rejected}"
    );
    assert_eq!(transport.requests().len(), 1);
}

async fn trailing_slash_base_keeps_one_slash() {
    let (api, transport) = harness();
    transport.queue(200, &[], QUEUED);
    transport.queue(200, &[("openai-poll-after-ms", "0")], COMPLETED);
    let mut trailing = call(Facade::SyncRun);
    trailing.options.api_base = Some(String::from("https://example.test/v1/"));
    api.run_thread(trailing, &request()).await.unwrap();
    assert_eq!(transport.requests()[0].url, RUNS);
}

// The SDK reads openai-poll-after-ms on the first non-terminal poll and reuses it for every later one.
async fn poll_intervals() {
    let started = Instant::now();
    let (api, transport) = harness();
    transport.queue(200, &[], QUEUED);
    transport.queue(200, &[("openai-poll-after-ms", "250")], IN_PROGRESS);
    transport.queue(200, &[("openai-poll-after-ms", "900")], IN_PROGRESS);
    transport.queue(200, &[], COMPLETED);
    let outcome = api
        .run_thread(call(Facade::SyncRun), &request())
        .await
        .unwrap();
    let RunInvocation::Polled { run, .. } = outcome else {
        panic!("expected a polled invocation");
    };
    assert_eq!(run.status, RunStatus::Completed);
    assert_eq!(transport.requests().len(), 4);
    assert_eq!(started.elapsed(), Duration::from_millis(500));

    let default_started = Instant::now();
    let (default_api, default_transport) = harness();
    default_transport.queue(200, &[], QUEUED);
    default_transport.queue(200, &[], IN_PROGRESS);
    default_transport.queue(200, &[("openai-poll-after-ms", "5")], IN_PROGRESS);
    default_transport.queue(200, &[], COMPLETED);
    default_api
        .run_thread(call(Facade::SyncRun), &request())
        .await
        .unwrap();
    assert_eq!(default_started.elapsed(), Duration::from_millis(2000));
}

async fn poll_interval_failures() {
    let (api, transport) = harness();
    transport.queue(200, &[], QUEUED);
    transport.queue_opaque_header(200, "openai-poll-after-ms", b"\xff", IN_PROGRESS);
    let error = api
        .run_thread(call(Facade::SyncRun), &request())
        .await
        .err()
        .unwrap();
    assert!(
        matches!(error, ThreadRunsError::PollAfterHeader(_)),
        "{error}"
    );
    assert_eq!(transport.requests().len(), 2);

    let (odd_api, odd_transport) = harness();
    odd_transport.queue(200, &[], QUEUED);
    odd_transport.queue(200, &[("openai-poll-after-ms", "soon")], IN_PROGRESS);
    let odd_error = odd_api
        .run_thread(call(Facade::SyncRun), &request())
        .await
        .err()
        .unwrap();
    assert!(
        matches!(odd_error, ThreadRunsError::PollAfter { .. }),
        "{odd_error}"
    );
    assert_eq!(odd_transport.requests().len(), 2);
}

// The client timeout is the deadline on every request instead of the 600s default.
async fn configured_timeout_is_the_deadline() {
    let (api, transport) = harness();
    transport.queue(200, &[], QUEUED);
    transport.queue(200, &[("openai-poll-after-ms", "0")], COMPLETED);
    let mut timed = call(Facade::SyncRun);
    timed.options.timeout = Some(Duration::from_millis(2500));
    api.run_thread(timed, &request()).await.unwrap();
    let deadlines: Vec<_> = transport
        .requests()
        .into_iter()
        .map(|seen| seen.deadline)
        .collect();
    assert_eq!(deadlines, vec![Duration::from_millis(2500); 2]);

    let (stream_api, stream_transport) = harness();
    stream_transport.queue(200, &[("content-type", "text/event-stream")], &sse());
    let mut streaming = call(Facade::AsyncStream);
    streaming.options.timeout = Some(Duration::from_millis(2500));
    let outcome = stream_api.run_thread(streaming, &request()).await.unwrap();
    let RunInvocation::Streaming(stream) = outcome else {
        panic!("expected a stream manager");
    };
    stream.consume().await.unwrap();
    assert_eq!(
        stream_transport.requests()[0].deadline,
        Duration::from_millis(2500)
    );
}

async fn requires_action() {
    let (_, run) = poll_state(REQUIRES_ACTION, true).await;
    assert_unchanged(&run, REQUIRES_ACTION);
    let tool_calls = run
        .required_action
        .as_ref()
        .map(|action| action.submit_tool_outputs.tool_calls.len());
    assert_eq!(tool_calls, Some(1));
}

async fn failed_run() {
    let (_, run) = poll_state(FAILED, false).await;
    assert_unchanged(&run, FAILED);
    assert_eq!(
        run.last_error.as_ref().map(|error| error.message.as_str()),
        Some("boom")
    );
    let (api, transport) = harness();
    transport.queue(200, &[], QUEUED);
    transport.queue(500, &[], ERROR_BODY);
    let error = api
        .run_thread(call(Facade::SyncRun), &request())
        .await
        .err()
        .unwrap();
    assert!(matches!(error, ThreadRunsError::Status(500, _)), "{error}");
    assert_eq!(transport.requests().len(), 2);
}

// Every SDK event family is yielded; unknown thread.* names and non-thread frames keep their payload.
fn families_body() -> String {
    let mut body = frame("thread.run.created", QUEUED);
    body.push_str(&frame(
        "thread.created",
        r#"{"id":"thread_a","object":"thread","created_at":1,"metadata":{}}"#,
    ));
    body.push_str(&frame("thread.run.step.created", STEP));
    body.push_str(&frame("thread.message.created", MESSAGE));
    body.push_str(&frame("thread.message.completed", MESSAGE));
    body.push_str(&frame("thread.message.delta", MESSAGE_DELTA));
    body.push_str(&frame("thread.run.step.completed", STEP));
    body.push_str(&frame("thread.future.thing", FUTURE));
    body.push_str(&frame("thread.run.completed", COMPLETED));
    body.push_str("data: [DONE]\n\n");
    body
}

async fn stream_event_families() {
    let (api, transport) = harness();
    let body = families_body();
    transport.queue(200, &[("content-type", "text/event-stream")], &body);
    let outcome = api
        .run_thread(call(Facade::SyncStream), &request())
        .await
        .unwrap();
    let RunInvocation::Streaming(stream) = outcome else {
        panic!("expected a stream manager");
    };
    let events = stream.consume().await.unwrap();
    let names: Vec<_> = events.iter().map(RunEvent::name).collect();
    assert_eq!(
        names,
        vec![
            "thread.run.created",
            "thread.created",
            "thread.run.step.created",
            "thread.message.created",
            "thread.message.completed",
            "thread.message.delta",
            "thread.run.step.completed",
            "thread.future.thing",
            "thread.run.completed",
        ]
    );
    let future = serde_json::to_value(&events[7]).unwrap();
    assert_eq!(
        future,
        parse(r#"{"event":"thread.future.thing","data":{"thing":true}}"#)
    );
}

// A comment-only frame is not an event.
async fn stream_comment_frames() {
    let body = format!(
        ": keepalive\n\n{}{}: ping\n\ndata: [DONE]\n\n",
        frame("thread.run.created", QUEUED),
        frame("thread.run.completed", COMPLETED),
    );
    assert_eq!(consume_body(&body).await.len(), 2);
}

// A tail without its blank line, complete or cut mid-JSON, is never dispatched.
async fn stream_tail_edges() {
    let unterminated = format!("event: thread.run.completed\ndata: {COMPLETED}");
    let tail = format!("{}{unterminated}", frame("thread.run.created", QUEUED));
    assert_eq!(consume_body(&tail).await.len(), 1);
    let cut = format!(
        "{}event: thread.run.completed\ndata: {{\"id\":",
        frame("thread.run.created", QUEUED)
    );
    assert_eq!(consume_body(&cut).await.len(), 1);
}

async fn consume_body(body: &str) -> Vec<RunEvent> {
    let (api, transport) = harness();
    transport.queue(200, &[("content-type", "text/event-stream")], body);
    let outcome = api
        .run_thread(call(Facade::SyncStream), &request())
        .await
        .unwrap();
    let RunInvocation::Streaming(stream) = outcome else {
        panic!("expected a stream manager");
    };
    stream.consume().await.unwrap()
}

async fn stream_stops_at_done() {
    let body = format!(
        "{}data: [DONE]\n\n{}",
        frame("thread.run.created", QUEUED),
        frame("thread.run.completed", COMPLETED),
    );
    let events = consume_body(&body).await;
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].name(), "thread.run.created");
}

async fn stream_provider_error() {
    let body = format!(
        "{}data: {{\"error\":{{\"message\":\"boom\",\"type\":\"t\",\"code\":\"c\"}}}}\n\n",
        frame("thread.run.created", QUEUED),
    );
    let (api, transport) = harness();
    transport.queue(200, &[("content-type", "text/event-stream")], &body);
    let recorder = Arc::new(RecordingHandler::default());
    let concrete: Arc<RecordingHandler> = Arc::clone(&recorder);
    let supplied: Arc<dyn RunEventHandler> = concrete;
    let mut supplied_call = call(Facade::SyncStream);
    supplied_call.handler = Some(supplied);
    let outcome = api.run_thread(supplied_call, &request()).await.unwrap();
    let RunInvocation::Streaming(stream) = outcome else {
        panic!("expected a stream manager");
    };
    let error = stream.consume().await.err().unwrap();
    assert!(
        matches!(error, ThreadRunsError::ProviderError { .. }),
        "{error}"
    );
    assert_eq!(
        recorder.take().join(","),
        "thread.run.created,exception:stream_error,end"
    );
}

// The event reaches the handler before the next chunk is released, so delivery is incremental.
async fn stream_delivers_per_chunk() {
    let (api, transport) = harness();
    let sender = transport.gate();
    let released = format!(
        "{}data: [DONE]\n\n",
        frame("thread.run.completed", COMPLETED)
    );
    let handler = Arc::new(GatedHandler {
        calls: Mutex::new(Vec::new()),
        sender: Mutex::new(Some(sender.clone())),
        payload: released.into_bytes(),
    });
    let concrete: Arc<GatedHandler> = Arc::clone(&handler);
    let supplied: Arc<dyn RunEventHandler> = concrete;
    let mut supplied_call = call(Facade::SyncStream);
    supplied_call.handler = Some(supplied);
    sender
        .try_send(format!("event: thread.run.created\ndata: {QUEUED}\n\n").into_bytes())
        .unwrap();
    let outcome = api.run_thread(supplied_call, &request()).await.unwrap();
    let RunInvocation::Streaming(stream) = outcome else {
        panic!("expected a stream manager");
    };
    let events = stream.consume().await.unwrap();
    assert_eq!(events.len(), 2);
    assert_eq!(
        handler.take().join(","),
        "thread.run.created,released,thread.run.completed,end"
    );
}

async fn stream_incremental_events() {
    let (api, transport) = harness();
    transport.queue(200, &[("content-type", "text/event-stream")], &sse());
    let outcome = api
        .run_thread(call(Facade::SyncStream), &request())
        .await
        .unwrap();
    let RunInvocation::Streaming(stream) = outcome else {
        panic!("expected a stream manager");
    };
    assert!(transport.requests().is_empty());
    let events = stream.consume().await.unwrap();
    assert_eq!(events.len(), 3);
    let created = parse(&format!(
        r#"{{"event":"thread.run.created","data":{QUEUED}}}"#
    ));
    let output = serde_json::to_value(&events[0]).unwrap();
    assert_eq!(output.get("event"), created.get("event"));
    let output_data = output.get("data").unwrap();
    assert_eq!(
        output_data.get("id"),
        Some(&Value::String(String::from("run_1")))
    );
    assert_eq!(
        output_data.get("status"),
        Some(&Value::String(String::from("queued")))
    );
    assert_eq!(events[2].name(), "thread.run.completed");
    assert_eq!(
        transport.requests(),
        vec![wire("POST", RUNS, Some(STREAM_BODY))]
    );

    stream_terminators_decode().await;
}

// CRLF is one terminator, not a line end plus a blank line that would cut the frame; a lone CR is a terminator too.
async fn stream_terminators_decode() {
    for terminator in ["\r\n", "\r"] {
        let (api, transport) = harness();
        let body = sse().replace('\n', terminator);
        transport.queue(200, &[("content-type", "text/event-stream")], &body);
        let outcome = api
            .run_thread(call(Facade::SyncStream), &request())
            .await
            .unwrap();
        let RunInvocation::Streaming(stream) = outcome else {
            panic!("expected a stream manager");
        };
        let events = stream.consume().await.unwrap();
        assert_eq!(events.len(), 3, "{terminator:?}");
        assert_eq!(
            transport.requests(),
            vec![wire("POST", RUNS, Some(STREAM_BODY))]
        );
    }
}

async fn instructions_tools_overrides() {
    let empty: RunRequest = serde_json::from_str(
        r#"{"thread_id":"thread_a","assistant_id":"asst_a","additional_instructions":null,"instructions":null,"metadata":null,"model":null,"tools":null}"#,
    )
    .unwrap();
    let (poll_api, poll_transport) = harness();
    poll_transport.queue(200, &[], QUEUED);
    poll_transport.queue(200, &[("openai-poll-after-ms", "0")], COMPLETED);
    poll_api
        .run_thread(call(Facade::SyncRun), &empty)
        .await
        .unwrap();
    let polled = poll_transport.requests();
    assert_eq!(polled[0].body, Some(parse(NULL_BODY)));
    assert!(polled[0].body.as_ref().unwrap().get("thread_id").is_none());

    let (stream_api, stream_transport) = harness();
    stream_transport.queue(200, &[("content-type", "text/event-stream")], &sse());
    let outcome = stream_api
        .run_thread(call(Facade::AsyncStream), &empty)
        .await
        .unwrap();
    let RunInvocation::Streaming(stream) = outcome else {
        panic!("expected a stream manager");
    };
    stream.consume().await.unwrap();
    let streamed = stream_transport.requests();
    assert_eq!(streamed[0].body, Some(parse(NULL_STREAM_BODY)));
    assert_eq!(
        streamed[0].body.as_ref().unwrap().get("model"),
        Some(&Value::Null)
    );
}

async fn dispatch_selections() {
    let selections = [
        (Facade::AsyncRun, ClientKind::Async, false),
        (Facade::SyncRun, ClientKind::Sync, false),
        (Facade::AsyncStream, ClientKind::Async, true),
        (Facade::SyncStream, ClientKind::Sync, true),
    ];
    for (facade, expected, streaming) in selections {
        let (api, transport) = harness();
        if streaming {
            transport.queue(200, &[("content-type", "text/event-stream")], &sse());
        } else {
            transport.queue(200, &[], QUEUED);
            transport.queue(200, &[("openai-poll-after-ms", "0")], COMPLETED);
        }
        let outcome = api.run_thread(call(facade), &request()).await.unwrap();
        match outcome {
            RunInvocation::Polled { kind, .. } => {
                assert!(!streaming);
                assert_eq!(kind, expected);
            }
            RunInvocation::Streaming(stream) => {
                assert!(streaming);
                assert_eq!(stream.kind, expected);
                stream.consume().await.unwrap();
            }
        }
    }
}

async fn stream_event_handler() {
    let (default_api, default_transport) = harness();
    default_transport.queue(200, &[("content-type", "text/event-stream")], &sse());
    let plain = default_api
        .run_thread(call(Facade::SyncStream), &request())
        .await
        .unwrap();
    let RunInvocation::Streaming(default_stream) = plain else {
        panic!("expected a stream manager");
    };
    assert!(default_stream.handler.is_none());
    assert!(default_transport.requests().is_empty());
    let default_events = default_stream.consume().await.unwrap();
    assert_eq!(default_events.len(), 3);
    assert_eq!(default_transport.requests(), vec![stream_wire(false)]);

    let (sink_api, sink_transport) = harness();
    sink_transport.queue(200, &[("content-type", "text/event-stream")], &sse());
    let recorder = Arc::new(RecordingHandler::default());
    let concrete: Arc<RecordingHandler> = Arc::clone(&recorder);
    let supplied: Arc<dyn RunEventHandler> = concrete;
    let mut supplied_call = call(Facade::AsyncStream);
    supplied_call.handler = Some(supplied);
    let supplied_outcome = sink_api
        .run_thread(supplied_call, &request())
        .await
        .unwrap();
    let RunInvocation::Streaming(sink_stream) = supplied_outcome else {
        panic!("expected a stream manager");
    };
    assert!(sink_stream.handler.is_some());
    assert_eq!(sink_stream.kind, ClientKind::Async);
    let sink_events = sink_stream.consume().await.unwrap();
    assert_eq!(sink_events.len(), 3);
    assert_eq!(
        recorder.take().join(","),
        "thread.run.created,thread.run.in_progress,thread.run.completed,end"
    );
    assert_eq!(sink_transport.requests(), vec![stream_wire(true)]);
}

async fn assistants_beta_default() {
    let (api, transport) = harness();
    transport.queue(200, &[], QUEUED);
    transport.queue(200, &[("openai-poll-after-ms", "0")], COMPLETED);
    api.run_thread(call(Facade::SyncRun), &request())
        .await
        .unwrap();
    let (streaming_api, streaming_transport) = harness();
    streaming_transport.queue(200, &[("content-type", "text/event-stream")], &sse());
    let outcome = streaming_api
        .run_thread(call(Facade::AsyncStream), &request())
        .await
        .unwrap();
    let RunInvocation::Streaming(stream) = outcome else {
        panic!("expected a stream manager");
    };
    stream.consume().await.unwrap();
    let betas: Vec<_> = transport
        .requests()
        .into_iter()
        .chain(streaming_transport.requests())
        .map(|seen| seen.beta)
        .collect();
    assert_eq!(betas, vec![Some(String::from("assistants=v2")); 3]);
}

// The cases share one production entry point; each names the outcome it checks.
#[rstest]
#[case::run_poll_completion(Scenario::RunPollCompletion)]
#[case::requires_action(Scenario::RequiresAction)]
#[case::failed_run(Scenario::FailedRun)]
#[case::stream_incremental_events(Scenario::StreamIncrementalEvents)]
#[case::instructions_tools_overrides(Scenario::InstructionsToolsOverrides)]
#[case::run_dispatch(Scenario::RunDispatch)]
#[case::stream_event_handler(Scenario::StreamEventHandler)]
#[case::assistants_beta_default(Scenario::AssistantsBetaDefault)]
#[tokio::test(start_paused = true)]
async fn thread_runs_cases(#[case] scenario: Scenario) {
    match scenario {
        Scenario::RunPollCompletion => run_poll_completion().await,
        Scenario::RequiresAction => requires_action().await,
        Scenario::FailedRun => failed_run().await,
        Scenario::StreamIncrementalEvents => {
            stream_incremental_events().await;
            stream_event_families().await;
            stream_comment_frames().await;
            stream_tail_edges().await;
            stream_stops_at_done().await;
            stream_provider_error().await;
            stream_delivers_per_chunk().await;
        }
        Scenario::InstructionsToolsOverrides => instructions_tools_overrides().await,
        Scenario::RunDispatch => dispatch_selections().await,
        Scenario::StreamEventHandler => stream_event_handler().await,
        Scenario::AssistantsBetaDefault => assistants_beta_default().await,
    }
}

#[derive(Debug)]
enum Scenario {
    RunPollCompletion,
    RequiresAction,
    FailedRun,
    StreamIncrementalEvents,
    InstructionsToolsOverrides,
    RunDispatch,
    StreamEventHandler,
    AssistantsBetaDefault,
}
