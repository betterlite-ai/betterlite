#![cfg(test)]

use std::sync::Arc;

use betterlite_http::client_kind::ClientKind;
use betterlite_models::controls::json_value::JsonValue;
use betterlite_openai::thread_runs::error::ThreadRunsError;
use betterlite_openai::thread_runs::operations::RunDispatch;
use betterlite_openai::thread_runs::records::RunRequest;
use futures_util::StreamExt as _;

use crate::openai_assistant_dispatch::OpenAiAssistantResult;
use crate::openai_assistant_dispatch::call::{OpenAiAssistantOperation, RunsCall};
use crate::openai_assistant_dispatch::error::OpenAiAssistantError;
use crate::openai_assistant_dispatch::reply::RunOutcome;

use super::support::{BASE, ERROR_BODY, Recorder, RecordingLogger, options, run_operation};

const RUN: &str = r#"{"id":"run_1","object":"thread.run","created_at":1700000000,"assistant_id":"asst_1","thread_id":"thread_1","status":"completed","instructions":"base","model":"gpt-4o-mini","parallel_tool_calls":true,"tools":[]}"#;
const RUN_BODY: &str = r#"{"assistant_id":"asst_1","additional_instructions":null,"instructions":"base","metadata":{"tag":"x"},"model":"gpt-4o-mini","stream":false,"tools":[{"type":"code_interpreter"}]}"#;
const STREAM_BODY: &str = r#"{"assistant_id":"asst_1","additional_instructions":null,"instructions":"base","metadata":{"tag":"x"},"model":"gpt-4o-mini","stream":true,"tools":[{"type":"code_interpreter"}]}"#;
const RUN_REQUEST: &str = r#"{"thread_id":"thread_1","assistant_id":"asst_1","instructions":"base","metadata":{"tag":"x"},"model":"gpt-4o-mini","tools":[{"type":"code_interpreter"}]}"#;

fn runs_operation(dispatch: RunDispatch) -> OpenAiAssistantOperation {
    let request: RunRequest = serde_json::from_str(RUN_REQUEST).unwrap();
    OpenAiAssistantOperation::Runs(RunsCall {
        options: options(),
        dispatch,
        request,
    })
}

fn stream_body() -> String {
    format!(
        "event: thread.run.created\ndata: {RUN}\n\nevent: thread.run.completed\ndata: {RUN}\n\ndata: [DONE]\n\n"
    )
}

async fn runs_buffered() {
    let transport = Arc::new(Recorder::default());
    transport.queue(200, RUN);
    transport.queue(200, RUN);
    let mut logger = RecordingLogger::default();
    let call = runs_operation(RunDispatch::default());
    let outcome = run_operation(&transport, &mut logger, call).await.unwrap();
    let OpenAiAssistantResult::Runs(run) = outcome else {
        panic!("a buffered run did not return a polled reply")
    };
    let RunOutcome::Polled(kind, reply) = *run else {
        panic!()
    };
    assert_eq!(kind, ClientKind::Sync);
    assert_eq!(reply.id, "run_1");
    assert_eq!(reply.status, "completed");
    let seen = transport.seen.lock().unwrap().clone();
    assert_eq!(seen.len(), 2);
    assert_eq!(seen[0].method, "POST");
    assert_eq!(seen[0].url, format!("{BASE}/threads/thread_1/runs"));
    let expected: JsonValue = serde_json::from_str(RUN_BODY).unwrap();
    assert_eq!(seen[0].body.as_ref(), Some(&expected));
    assert_eq!(seen[1].method, "GET");
    assert_eq!(seen[1].url, format!("{BASE}/threads/thread_1/runs/run_1"));
}

async fn runs_async_facade() {
    let transport = Arc::new(Recorder::default());
    transport.queue(200, RUN);
    transport.queue(200, RUN);
    let mut logger = RecordingLogger::default();
    let dispatch = RunDispatch {
        arun_thread: true,
        stream: false,
    };
    let call = runs_operation(dispatch);
    let outcome = run_operation(&transport, &mut logger, call).await.unwrap();
    let OpenAiAssistantResult::Runs(run) = outcome else {
        panic!("the async facade did not return a polled reply")
    };
    let RunOutcome::Polled(kind, reply) = *run else {
        panic!()
    };
    assert_eq!(kind, ClientKind::Asynchronous);
    assert_eq!(reply.status, "completed");
    let seen = transport.seen.lock().unwrap().clone();
    assert_eq!(seen.len(), 2);
    let expected: JsonValue = serde_json::from_str(RUN_BODY).unwrap();
    assert_eq!(seen[0].body.as_ref(), Some(&expected));
}

async fn runs_streaming() {
    let transport = Arc::new(Recorder::default());
    *transport.stream.lock().unwrap() = Some((200, stream_body()));
    let mut logger = RecordingLogger::default();
    let dispatch = RunDispatch {
        arun_thread: false,
        stream: true,
    };
    let call = runs_operation(dispatch);
    let outcome = run_operation(&transport, &mut logger, call).await.unwrap();
    let OpenAiAssistantResult::Runs(run) = outcome else {
        panic!("a streaming run did not stay a stream")
    };
    let RunOutcome::Streaming(kind, mut events) = *run else {
        panic!("a streaming run did not stay a stream")
    };
    assert_eq!(kind, ClientKind::Sync);
    assert!(transport.seen.lock().unwrap().is_empty());
    let mut names: Vec<String> = Vec::new();
    while let Some(event) = events.next().await {
        names.push(event.unwrap().name().to_owned());
    }
    assert_eq!(names, ["thread.run.created", "thread.run.completed"]);
    let seen = transport.seen.lock().unwrap().clone();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].method, "POST");
    let expected: JsonValue = serde_json::from_str(STREAM_BODY).unwrap();
    assert_eq!(seen[0].body.as_ref(), Some(&expected));
    assert_eq!(seen[0].beta.as_deref(), Some("assistants=v2"));
}

// LiteLLM returns the stream manager unopened, so a refused open reaches the caller only when it reads the stream.
async fn runs_stream_open_failure() {
    let transport = Arc::new(Recorder::default());
    *transport.stream.lock().unwrap() = Some((400, ERROR_BODY.to_owned()));
    let mut logger = RecordingLogger::default();
    let dispatch = RunDispatch {
        arun_thread: true,
        stream: true,
    };
    let call = runs_operation(dispatch);
    let outcome = run_operation(&transport, &mut logger, call).await.unwrap();
    let OpenAiAssistantResult::Runs(run) = outcome else {
        panic!("a refused stream did not stay a stream")
    };
    let RunOutcome::Streaming(kind, mut events) = *run else {
        panic!("a refused stream did not stay a stream")
    };
    assert_eq!(kind, ClientKind::Asynchronous);
    assert!(transport.seen.lock().unwrap().is_empty());
    let Some(Err(ThreadRunsError::Status(400, body))) = events.next().await else {
        panic!("the refused open did not come out of the stream")
    };
    assert_eq!(body, ERROR_BODY.as_bytes());
    assert!(events.next().await.is_none());
    assert_eq!(transport.seen.lock().unwrap().len(), 1);
}

async fn runs_leaf_error() {
    let transport = Arc::new(Recorder::default());
    transport.queue(500, ERROR_BODY);
    let mut logger = RecordingLogger::default();
    let call = runs_operation(RunDispatch::default());
    let Err(error) = run_operation(&transport, &mut logger, call).await else {
        panic!("the run status error did not surface")
    };
    assert!(matches!(
        error,
        OpenAiAssistantError::Runs(ThreadRunsError::Status(500, _))
    ));
}

pub(super) async fn case() {
    runs_buffered().await;
    runs_async_facade().await;
    runs_streaming().await;
    runs_stream_open_failure().await;
    runs_leaf_error().await;
}
