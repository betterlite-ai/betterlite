#![cfg(test)]

use std::sync::Arc;

use rstest::rstest;
use serde_json::Value;

use crate::assistants::client::ClientKind;
use crate::thread_runs::error::ThreadRunsError;
use crate::thread_runs::operations::{RunCall, RunDispatch, RunInvocation};
use crate::thread_runs::records::{RunReply, RunRequest};
use crate::thread_runs::stream::RunEventHandler;

use super::support::{
    COMPLETED, FAILED, IN_PROGRESS, KEY, QUEUED, REQUIRES_ACTION, Recorder, RecordingHandler, Seen,
    harness, options, parse, wire,
};

const RUNS: &str = "https://example.test/v1/threads/thread_a/runs";
const RUN_1: &str = "https://example.test/v1/threads/thread_a/runs/run_1";
const REQUEST: &str = r#"{"thread_id":"thread_a","assistant_id":"asst_a","additional_instructions":"extra","instructions":"base","metadata":{"tag":"x"},"model":"gpt-4o-mini","tools":[{"type":"code_interpreter"}]}"#;
const POLL_BODY: &str = r#"{"assistant_id":"asst_a","additional_instructions":"extra","instructions":"base","metadata":{"tag":"x"},"model":"gpt-4o-mini","stream":false,"tools":[{"type":"code_interpreter"}]}"#;
const STREAM_BODY: &str = r#"{"assistant_id":"asst_a","additional_instructions":"extra","instructions":"base","metadata":{"tag":"x"},"model":"gpt-4o-mini","stream":true,"tools":[{"type":"code_interpreter"}]}"#;
const NULL_BODY: &str = r#"{"assistant_id":"asst_a","additional_instructions":null,"instructions":null,"metadata":null,"model":null,"stream":false,"tools":null}"#;
const NULL_STREAM_BODY: &str = r#"{"assistant_id":"asst_a","additional_instructions":null,"instructions":null,"metadata":null,"model":null,"stream":true,"tools":null}"#;
const ERROR_BODY: &str = r#"{"error":{"message":"no run","type":"invalid_request_error"}}"#;

fn request() -> RunRequest {
    serde_json::from_str(REQUEST).unwrap()
}

fn call(arun_thread: Option<bool>, stream: Option<bool>) -> RunCall<Recorder> {
    RunCall {
        options: options(KEY),
        client: None,
        dispatch: RunDispatch {
            arun_thread,
            stream,
        },
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

fn sse() -> String {
    format!(
        "event: thread.run.created\ndata: {QUEUED}\n\nevent: thread.run.in_progress\ndata: {IN_PROGRESS}\n\nevent: thread.run.completed\ndata: {COMPLETED}\n\ndata: [DONE]\n\n"
    )
}

async fn poll_to(state: &str, arun_thread: bool) -> (Vec<Seen>, RunReply) {
    let (api, transport) = harness();
    transport.queue(200, &[], QUEUED);
    transport.queue(200, &[("openai-poll-after-ms", "0")], state);
    let outcome = api
        .run_thread(call(Some(arun_thread), Some(false)), &request())
        .await
        .unwrap();
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

// Both buffered paths hand back the created-and-polled run, whatever terminal state it reached.
async fn run_poll_completion() {
    for (state, arun_thread) in [
        (COMPLETED, false),
        (COMPLETED, true),
        (REQUIRES_ACTION, false),
        (FAILED, true),
    ] {
        let (seen, run) = poll_to(state, arun_thread).await;
        assert_eq!(serde_json::to_value(&run).unwrap(), parse(state));
        assert_eq!(seen, vec![wire("POST", RUNS, Some(POLL_BODY)), poll_wire()]);
    }
    let (api, transport) = harness();
    transport.queue(404, &[], ERROR_BODY);
    let error = api
        .run_thread(call(Some(false), Some(false)), &request())
        .await
        .err()
        .unwrap();
    assert!(
        matches!(error, ThreadRunsError::Status { status: 404, .. }),
        "{error}"
    );
    assert_eq!(transport.requests().len(), 1);
}

async fn requires_action() {
    let (_, run) = poll_to(REQUIRES_ACTION, true).await;
    assert_eq!(serde_json::to_value(&run).unwrap(), parse(REQUIRES_ACTION));
    let tool_calls = run
        .required_action
        .as_ref()
        .map(|action| action.submit_tool_outputs.tool_calls.len());
    assert_eq!(tool_calls, Some(1));
}

async fn failed_run() {
    let (_, run) = poll_to(FAILED, false).await;
    assert_eq!(serde_json::to_value(&run).unwrap(), parse(FAILED));
    assert_eq!(
        run.last_error.as_ref().map(|error| error.message.as_str()),
        Some("boom")
    );
    let (api, transport) = harness();
    transport.queue(200, &[], QUEUED);
    transport.queue(500, &[], ERROR_BODY);
    let error = api
        .run_thread(call(Some(false), Some(false)), &request())
        .await
        .err()
        .unwrap();
    assert!(
        matches!(error, ThreadRunsError::Status { status: 500, .. }),
        "{error}"
    );
    assert_eq!(transport.requests().len(), 2);
}

async fn stream_incremental_events() {
    let (api, transport) = harness();
    transport.queue(200, &[("content-type", "text/event-stream")], &sse());
    let outcome = api
        .run_thread(call(Some(false), Some(true)), &request())
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
    assert_eq!(serde_json::to_value(&events[0]).unwrap(), created);
    let completed = parse(&format!(
        r#"{{"event":"thread.run.completed","data":{COMPLETED}}}"#
    ));
    assert_eq!(serde_json::to_value(&events[2]).unwrap(), completed);
    assert_eq!(
        transport.requests(),
        vec![wire("POST", RUNS, Some(STREAM_BODY))]
    );
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
        .run_thread(call(Some(false), Some(false)), &empty)
        .await
        .unwrap();
    let polled = poll_transport.requests();
    assert_eq!(polled[0].body, Some(parse(NULL_BODY)));
    assert!(polled[0].body.as_ref().unwrap().get("thread_id").is_none());

    let (stream_api, stream_transport) = harness();
    stream_transport.queue(200, &[("content-type", "text/event-stream")], &sse());
    let outcome = stream_api
        .run_thread(call(Some(true), Some(true)), &empty)
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
        (Some(true), Some(false), ClientKind::Async, false),
        (Some(false), Some(false), ClientKind::Sync, false),
        (None, None, ClientKind::Sync, false),
        (Some(true), Some(true), ClientKind::Async, true),
        (Some(false), Some(true), ClientKind::Sync, true),
        (None, Some(true), ClientKind::Sync, true),
    ];
    for (arun_thread, stream_flag, expected, streaming) in selections {
        let (api, transport) = harness();
        if streaming {
            transport.queue(200, &[("content-type", "text/event-stream")], &sse());
        } else {
            transport.queue(200, &[], QUEUED);
            transport.queue(200, &[("openai-poll-after-ms", "0")], COMPLETED);
        }
        let outcome = api
            .run_thread(call(arun_thread, stream_flag), &request())
            .await
            .unwrap();
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
        .run_thread(call(Some(false), Some(true)), &request())
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
    let mut supplied_call = call(Some(true), Some(true));
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
    api.run_thread(call(Some(false), Some(false)), &request())
        .await
        .unwrap();
    let (streaming_api, streaming_transport) = harness();
    streaming_transport.queue(200, &[("content-type", "text/event-stream")], &sse());
    let outcome = streaming_api
        .run_thread(call(Some(true), Some(true)), &request())
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
#[tokio::test]
async fn thread_runs_cases(#[case] scenario: Scenario) {
    match scenario {
        Scenario::RunPollCompletion => run_poll_completion().await,
        Scenario::RequiresAction => requires_action().await,
        Scenario::FailedRun => failed_run().await,
        Scenario::StreamIncrementalEvents => stream_incremental_events().await,
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
