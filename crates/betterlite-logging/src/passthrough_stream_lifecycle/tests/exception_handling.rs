#![cfg(test)]

use std::sync::Arc;

use futures_util::StreamExt as _;
use indexmap::IndexMap;

use crate::passthrough_stream_lifecycle::chunk_processor::chunk_processor;
use crate::passthrough_stream_lifecycle::ports::StreamJobKind;
use crate::passthrough_stream_lifecycle::support::{
    Harness, RecordedFailure, failing_chunks, failure, json, object, ok_chunks, processor_call,
    recorded,
};
use crate::passthrough_stream_logging::types::EndpointType;

#[tokio::test]
async fn an_upstream_error_schedules_the_failure_and_never_logs_success() {
    let harness = Harness::new(&["2024-01-01T00:00:01+00:00"]);
    let lifecycle = harness.lifecycle(false);
    let call = processor_call(
        Arc::clone(&lifecycle),
        object(&[], IndexMap::new()),
        EndpointType::OpenAi,
    );
    let collected: Vec<_> =
        chunk_processor(call, failing_chunks(vec![b"alpha".to_vec()], failure()))
            .collect()
            .await;
    assert_eq!(collected.len(), 2);
    assert_eq!(collected[0].as_ref().unwrap(), b"alpha");
    assert_eq!(
        recorded(collected[1].as_ref().unwrap_err()),
        recorded(&failure())
    );
    assert_eq!(
        harness.failure_dispatch.failures(),
        vec![RecordedFailure {
            class: "ScriptedReadError".to_owned(),
            message: "the upstream read failed".to_owned(),
            traceback: "Traceback: scripted".to_owned(),
        }]
    );
    assert_eq!(harness.queue.kinds(), vec![StreamJobKind::Failure]);
    assert_eq!(harness.binder.calls(), 0);
    assert!(harness.route.calls().is_empty());
}

#[tokio::test]
async fn the_failure_is_scheduled_before_the_error_reaches_the_caller() {
    let harness = Harness::new(&["2024-01-01T00:00:01+00:00"]);
    let lifecycle = harness.lifecycle(false);
    let call = processor_call(
        Arc::clone(&lifecycle),
        object(&[], IndexMap::new()),
        EndpointType::OpenAi,
    );
    let mut stream = Box::pin(chunk_processor(
        call,
        failing_chunks(vec![b"alpha".to_vec()], failure()),
    ));
    assert_eq!(stream.next().await.unwrap().unwrap(), b"alpha");
    assert_eq!(
        recorded(&stream.next().await.unwrap().unwrap_err()),
        recorded(&failure())
    );
    assert_eq!(harness.failure_dispatch.failures().len(), 1);
}

#[tokio::test]
async fn an_upstream_error_after_status_400_is_not_scheduled() {
    let harness = Harness::new(&["2024-01-01T00:00:01+00:00"]);
    let lifecycle = harness.lifecycle(false);
    let mut call = processor_call(
        Arc::clone(&lifecycle),
        object(&[], IndexMap::new()),
        EndpointType::OpenAi,
    );
    call.status_code = 400;
    let collected: Vec<_> =
        chunk_processor(call, failing_chunks(vec![b"alpha".to_vec()], failure()))
            .collect()
            .await;
    assert_eq!(collected.len(), 2);
    assert_eq!(
        recorded(collected[1].as_ref().unwrap_err()),
        recorded(&failure())
    );
    assert!(harness.failure_dispatch.failures().is_empty());
    assert!(harness.queue.kinds().is_empty());
    assert_eq!(harness.binder.calls(), 0);
}

#[tokio::test]
async fn the_partial_bytes_reach_the_anthropic_usage_recovery() {
    let harness = Harness::new(&["2024-01-01T00:00:02+00:00"]);
    let lifecycle = harness.lifecycle(false);
    let call = processor_call(
        Arc::clone(&lifecycle),
        object(&[], IndexMap::new()),
        EndpointType::Anthropic,
    );
    drop(
        chunk_processor(
            call,
            failing_chunks(vec![b"event: message_start\n\n".to_vec()], failure()),
        )
        .collect::<Vec<_>>()
        .await,
    );
    assert_eq!(
        *harness.anthropic.calls.lock().unwrap(),
        vec![(json("{}"), vec![b"event: message_start\n\n".to_vec()])]
    );
    assert_eq!(harness.queue.kinds(), vec![StreamJobKind::Failure]);
}

#[tokio::test]
async fn a_disconnect_logs_the_partial_stream_as_success() {
    let harness = Harness::new(&["2024-01-01T00:00:01+00:00"]);
    let lifecycle = harness.lifecycle(false);
    let call = processor_call(
        Arc::clone(&lifecycle),
        object(&[], IndexMap::new()),
        EndpointType::OpenAi,
    );
    let mut stream = Box::pin(chunk_processor(
        call,
        ok_chunks(vec![b"alpha".to_vec(), b"beta".to_vec()]),
    ));
    assert_eq!(stream.next().await.unwrap().unwrap(), b"alpha");
    drop(stream);
    assert_eq!(harness.queue.kinds(), vec![StreamJobKind::Success]);
    assert_eq!(harness.binder.calls(), 1);
    assert!(harness.failure_dispatch.failures().is_empty());
    let job = harness.queue.take_all().into_iter().next().unwrap();
    job.work.await;
    assert_eq!(harness.route.calls()[0].raw_bytes, vec![b"alpha".to_vec()]);
}

#[tokio::test]
async fn a_disconnect_with_status_400_logs_nothing() {
    let harness = Harness::new(&["2024-01-01T00:00:01+00:00"]);
    let lifecycle = harness.lifecycle(false);
    let mut call = processor_call(
        Arc::clone(&lifecycle),
        object(&[], IndexMap::new()),
        EndpointType::OpenAi,
    );
    call.status_code = 400;
    let mut stream = Box::pin(chunk_processor(
        call,
        ok_chunks(vec![b"alpha".to_vec(), b"beta".to_vec()]),
    ));
    assert_eq!(stream.next().await.unwrap().unwrap(), b"alpha");
    drop(stream);
    assert!(harness.queue.kinds().is_empty());
    assert_eq!(harness.binder.calls(), 0);
    assert!(harness.failure_dispatch.failures().is_empty());
}

#[tokio::test]
async fn an_error_before_any_chunk_logs_no_success() {
    let harness = Harness::new(&["2024-01-01T00:00:01+00:00"]);
    let lifecycle = harness.lifecycle(false);
    let call = processor_call(
        Arc::clone(&lifecycle),
        object(&[], IndexMap::new()),
        EndpointType::OpenAi,
    );
    let collected: Vec<_> = chunk_processor(call, failing_chunks(Vec::new(), failure()))
        .collect()
        .await;
    assert_eq!(collected.len(), 1);
    assert_eq!(
        recorded(collected[0].as_ref().unwrap_err()),
        recorded(&failure())
    );
    assert_eq!(harness.queue.kinds(), vec![StreamJobKind::Failure]);
    assert_eq!(harness.binder.calls(), 0);
    assert!(harness.route.calls().is_empty());
}
