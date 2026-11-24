#![cfg(test)]

use std::sync::Arc;

use futures_util::StreamExt as _;

use crate::passthrough_stream_lifecycle::chunk_processor::chunk_processor;
use crate::passthrough_stream_lifecycle::support::{
    Harness, callback_bound, capture_logs, failing_chunks, failure, json, object, ok_chunks,
    processor_call, reservation_params,
};
use crate::passthrough_stream_logging::types::EndpointType;

#[tokio::test]
async fn a_successful_enqueue_binds_the_reservation() {
    let harness = Harness::new(&["2024-01-01T00:00:01+00:00"]);
    let lifecycle = harness.lifecycle(false);
    let logging_obj = object(&[], reservation_params());
    let call = processor_call(
        Arc::clone(&lifecycle),
        Arc::clone(&logging_obj),
        EndpointType::OpenAi,
    );
    let _: Vec<_> = chunk_processor(call, ok_chunks(vec![b"alpha".to_vec()]))
        .collect()
        .await;
    assert_eq!(harness.binder.calls(), 1);
    assert_eq!(callback_bound(logging_obj.as_ref()), Some(json("true")));
}

#[tokio::test]
async fn a_rejected_enqueue_never_binds_the_reservation() {
    let harness = Harness::new(&["2024-01-01T00:00:01+00:00"]).with_rejecting_queue();
    let lifecycle = harness.lifecycle(false);
    let logging_obj = object(&[], reservation_params());
    let call = processor_call(
        Arc::clone(&lifecycle),
        Arc::clone(&logging_obj),
        EndpointType::OpenAi,
    );
    let _: Vec<_> = chunk_processor(call, ok_chunks(vec![b"alpha".to_vec()]))
        .collect()
        .await;
    assert_eq!(harness.binder.calls(), 0);
    assert!(callback_bound(logging_obj.as_ref()).is_none());
}

#[tokio::test]
async fn a_failure_never_binds_the_reservation() {
    let harness = Harness::new(&["2024-01-01T00:00:01+00:00"]);
    let lifecycle = harness.lifecycle(false);
    let logging_obj = object(&[], reservation_params());
    let call = processor_call(
        Arc::clone(&lifecycle),
        Arc::clone(&logging_obj),
        EndpointType::OpenAi,
    );
    let _: Vec<_> = chunk_processor(call, failing_chunks(vec![b"alpha".to_vec()], failure()))
        .collect()
        .await;
    assert_eq!(harness.binder.calls(), 0);
    assert!(callback_bound(logging_obj.as_ref()).is_none());
}

#[tokio::test]
async fn a_disconnect_binds_after_the_partial_enqueue() {
    let harness = Harness::new(&["2024-01-01T00:00:01+00:00"]);
    let lifecycle = harness.lifecycle(false);
    let logging_obj = object(&[], reservation_params());
    let call = processor_call(
        Arc::clone(&lifecycle),
        Arc::clone(&logging_obj),
        EndpointType::OpenAi,
    );
    let mut stream = Box::pin(chunk_processor(
        call,
        ok_chunks(vec![b"alpha".to_vec(), b"beta".to_vec()]),
    ));
    assert_eq!(stream.next().await.unwrap().unwrap(), b"alpha");
    drop(stream);
    assert_eq!(harness.binder.calls(), 1);
    assert_eq!(callback_bound(logging_obj.as_ref()), Some(json("true")));
}

#[tokio::test]
async fn a_rejected_partial_enqueue_never_binds() {
    let harness = Harness::new(&["2024-01-01T00:00:01+00:00"]).with_rejecting_queue();
    let lifecycle = harness.lifecycle(false);
    let logging_obj = object(&[], reservation_params());
    let call = processor_call(
        Arc::clone(&lifecycle),
        Arc::clone(&logging_obj),
        EndpointType::OpenAi,
    );
    let (logs, guard) = capture_logs();
    let mut stream = Box::pin(chunk_processor(
        call,
        ok_chunks(vec![b"alpha".to_vec(), b"beta".to_vec()]),
    ));
    assert_eq!(stream.next().await.unwrap().unwrap(), b"alpha");
    drop(stream);
    drop(guard);
    assert_eq!(harness.binder.calls(), 0);
    assert!(callback_bound(logging_obj.as_ref()).is_none());
    let text = logs.text();
    assert!(
        text.contains("Error scheduling chunk_processor logging: QueueFull"),
        "{text}"
    );
}
