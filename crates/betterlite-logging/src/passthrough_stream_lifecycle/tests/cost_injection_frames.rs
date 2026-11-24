#![cfg(test)]

use std::sync::Arc;

use futures_util::StreamExt as _;
use indexmap::IndexMap;

use crate::passthrough_stream_lifecycle::chunk_processor::chunk_processor;
use crate::passthrough_stream_lifecycle::ports::StreamJobKind;
use crate::passthrough_stream_lifecycle::support::{
    Harness, completion_start_time, json, object, ok_chunks, processor_call,
};
use crate::passthrough_stream_logging::types::EndpointType;

#[tokio::test]
async fn complete_frames_are_transformed_and_the_tail_is_yielded_unchanged() {
    let harness = Harness::new(&["2024-01-01T00:00:01+00:00"]);
    let lifecycle = harness.lifecycle(true);
    let logging_obj = object(&[], IndexMap::new());
    let mut call = processor_call(
        Arc::clone(&lifecycle),
        Arc::clone(&logging_obj),
        EndpointType::OpenAi,
    );
    call.request_body = Some(json("{\"model\":\"gpt-4o\"}"));
    let chunks = vec![b"data: one\n\npa".to_vec(), b"rtial".to_vec()];
    let collected: Vec<_> = chunk_processor(call, ok_chunks(chunks.clone()))
        .collect()
        .await;
    let yielded: Vec<Vec<u8>> = collected.into_iter().map(|item| item.unwrap()).collect();
    assert_eq!(
        yielded,
        vec![b"injected:data: one\n\n".to_vec(), b"partial".to_vec()]
    );
    assert_eq!(
        harness.cost_injection.calls(),
        vec![(b"data: one\n\n".to_vec(), "gpt-4o".to_owned())]
    );
    assert!(completion_start_time(logging_obj.as_ref()).is_some());
    let job = harness.queue.take_all().into_iter().next().unwrap();
    job.work.await;
    assert_eq!(harness.route.calls()[0].raw_bytes, chunks);
}

#[tokio::test]
async fn every_complete_frame_in_one_chunk_is_injected_together() {
    let harness = Harness::new(&["2024-01-01T00:00:01+00:00"]);
    let lifecycle = harness.lifecycle(true);
    let mut call = processor_call(
        Arc::clone(&lifecycle),
        object(&[], IndexMap::new()),
        EndpointType::OpenAi,
    );
    call.request_body = Some(json("{\"model\":\"gpt-4o\"}"));
    let chunk = b"data: a\n\ndata: b\n\n".to_vec();
    let collected: Vec<_> = chunk_processor(call, ok_chunks(vec![chunk.clone()]))
        .collect()
        .await;
    let yielded: Vec<Vec<u8>> = collected.into_iter().map(|item| item.unwrap()).collect();
    assert_eq!(yielded, vec![b"injected:data: a\n\ndata: b\n\n".to_vec()]);
    assert_eq!(
        harness.cost_injection.calls(),
        vec![(chunk, "gpt-4o".to_owned())]
    );
}

#[tokio::test]
async fn an_incomplete_frame_never_reaches_the_injection() {
    let harness = Harness::new(&["2024-01-01T00:00:01+00:00"]);
    let lifecycle = harness.lifecycle(true);
    let mut call = processor_call(
        Arc::clone(&lifecycle),
        object(&[], IndexMap::new()),
        EndpointType::OpenAi,
    );
    call.request_body = Some(json("{\"model\":\"gpt-4o\"}"));
    let chunk = b"data: one".to_vec();
    let collected: Vec<_> = chunk_processor(call, ok_chunks(vec![chunk.clone()]))
        .collect()
        .await;
    let yielded: Vec<Vec<u8>> = collected.into_iter().map(|item| item.unwrap()).collect();
    assert_eq!(yielded, vec![chunk]);
    assert!(harness.cost_injection.calls().is_empty());
}

#[tokio::test]
async fn an_empty_chunk_is_buffered_but_yields_nothing() {
    let harness = Harness::new(&["2024-01-01T00:00:01+00:00"]);
    let lifecycle = harness.lifecycle(true);
    let mut call = processor_call(
        Arc::clone(&lifecycle),
        object(&[], IndexMap::new()),
        EndpointType::OpenAi,
    );
    call.request_body = Some(json("{\"model\":\"gpt-4o\"}"));
    let collected: Vec<_> = chunk_processor(call, ok_chunks(vec![Vec::new()]))
        .collect()
        .await;
    assert!(collected.is_empty());
    assert!(harness.cost_injection.calls().is_empty());
    assert_eq!(harness.queue.kinds(), vec![StreamJobKind::Success]);
    let job = harness.queue.take_all().into_iter().next().unwrap();
    job.work.await;
    assert_eq!(harness.route.calls()[0].raw_bytes, vec![Vec::<u8>::new()]);
}

#[tokio::test]
async fn injection_still_runs_when_the_status_forbids_logging() {
    let harness = Harness::new(&["2024-01-01T00:00:01+00:00"]);
    let lifecycle = harness.lifecycle(true);
    let mut call = processor_call(
        Arc::clone(&lifecycle),
        object(&[], IndexMap::new()),
        EndpointType::OpenAi,
    );
    call.request_body = Some(json("{\"model\":\"gpt-4o\"}"));
    call.status_code = 400;
    let collected: Vec<_> = chunk_processor(call, ok_chunks(vec![b"data: one\n\n".to_vec()]))
        .collect()
        .await;
    assert_eq!(collected.len(), 1);
    assert_eq!(harness.cost_injection.calls().len(), 1);
    assert!(harness.queue.kinds().is_empty());
}
