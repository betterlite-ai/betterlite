#![cfg(test)]

use std::sync::Arc;

use futures_util::StreamExt as _;
use indexmap::IndexMap;

use crate::passthrough_stream_lifecycle::chunk_processor::chunk_processor;
use crate::passthrough_stream_lifecycle::ports::StreamJobKind;
use crate::passthrough_stream_lifecycle::support::{
    Harness, completion_start_time, json, model_details, object, ok_chunks, processor_call, time,
};
use crate::passthrough_stream_logging::types::EndpointType;

#[tokio::test]
async fn every_chunk_is_forwarded_unchanged_and_buffered() {
    let harness = Harness::new(&["2024-01-01T00:00:01+00:00"]);
    let lifecycle = harness.lifecycle(false);
    let logging_obj = object(&[], IndexMap::new());
    let call = processor_call(
        Arc::clone(&lifecycle),
        Arc::clone(&logging_obj),
        EndpointType::OpenAi,
    );
    let chunks = vec![b"alpha".to_vec(), b"beta".to_vec()];
    let collected: Vec<_> = chunk_processor(call, ok_chunks(chunks.clone()))
        .collect()
        .await;
    let yielded: Vec<Vec<u8>> = collected.into_iter().map(|item| item.unwrap()).collect();
    assert_eq!(yielded, chunks);
    assert_eq!(harness.queue.kinds(), vec![StreamJobKind::Success]);
    let job = harness.queue.take_all().into_iter().next().unwrap();
    job.work.await;
    let routes = harness.route.calls();
    assert_eq!(routes.len(), 1);
    assert_eq!(routes[0].raw_bytes, chunks);
    assert_eq!(routes[0].url_route, "/v1/messages");
    assert_eq!(routes[0].endpoint_type, EndpointType::OpenAi);
    assert_eq!(routes[0].request_body, Some(json("{}")));
    assert_eq!(routes[0].model, None);
    assert_eq!(routes[0].start_time, time("2024-01-01T00:00:00+00:00"));
    assert_eq!(routes[0].end_time, time("2024-01-01T00:00:01+00:00"));
}

#[tokio::test]
async fn the_first_chunk_anchors_the_completion_start_time() {
    let harness = Harness::new(&["2024-01-01T00:00:01+00:00"]);
    let lifecycle = harness.lifecycle(false);
    let logging_obj = object(&[], IndexMap::new());
    let call = processor_call(
        Arc::clone(&lifecycle),
        Arc::clone(&logging_obj),
        EndpointType::OpenAi,
    );
    let chunks = vec![b"alpha".to_vec(), b"beta".to_vec()];
    drop(
        chunk_processor(call, ok_chunks(chunks))
            .collect::<Vec<_>>()
            .await,
    );
    assert_eq!(
        completion_start_time(logging_obj.as_ref()).as_deref(),
        Some("2024-01-01T00:00:01+00:00")
    );
    assert_eq!(harness.clock.calls(), 2);
}

#[tokio::test]
async fn an_empty_stream_logs_nothing() {
    let harness = Harness::new(&["2024-01-01T00:00:01+00:00"]);
    let lifecycle = harness.lifecycle(false);
    let logging_obj = object(&[], IndexMap::new());
    let call = processor_call(
        Arc::clone(&lifecycle),
        Arc::clone(&logging_obj),
        EndpointType::OpenAi,
    );
    let collected: Vec<_> = chunk_processor(call, ok_chunks(Vec::new())).collect().await;
    assert!(collected.is_empty());
    assert!(harness.queue.kinds().is_empty());
    assert_eq!(harness.binder.calls(), 0);
    assert!(harness.route.calls().is_empty());
    assert!(completion_start_time(logging_obj.as_ref()).is_none());
    assert_eq!(harness.clock.calls(), 0);
}

#[tokio::test]
async fn a_status_below_400_is_logged() {
    let harness = Harness::new(&["2024-01-01T00:00:01+00:00"]);
    let lifecycle = harness.lifecycle(false);
    let mut call = processor_call(
        Arc::clone(&lifecycle),
        object(&[], IndexMap::new()),
        EndpointType::OpenAi,
    );
    call.status_code = 399;
    drop(
        chunk_processor(call, ok_chunks(vec![b"alpha".to_vec()]))
            .collect::<Vec<_>>()
            .await,
    );
    assert_eq!(harness.queue.kinds(), vec![StreamJobKind::Success]);
    assert_eq!(harness.binder.calls(), 1);
}

#[tokio::test]
async fn a_status_of_400_is_not_logged() {
    let harness = Harness::new(&["2024-01-01T00:00:01+00:00"]);
    let lifecycle = harness.lifecycle(false);
    let logging_obj = object(&[], IndexMap::new());
    let mut call = processor_call(
        Arc::clone(&lifecycle),
        Arc::clone(&logging_obj),
        EndpointType::OpenAi,
    );
    call.status_code = 400;
    drop(
        chunk_processor(call, ok_chunks(vec![b"alpha".to_vec()]))
            .collect::<Vec<_>>()
            .await,
    );
    assert!(harness.queue.kinds().is_empty());
    assert_eq!(harness.binder.calls(), 0);
    assert!(harness.route.calls().is_empty());
    assert!(completion_start_time(logging_obj.as_ref()).is_some());
}

#[tokio::test]
async fn the_end_time_is_read_when_the_job_is_built() {
    let harness = Harness::new(&["2024-01-01T00:00:01+00:00", "2024-01-01T00:00:07+00:00"]);
    let lifecycle = harness.lifecycle(false);
    let logging_obj = object(&[], IndexMap::new());
    let mut call = processor_call(
        Arc::clone(&lifecycle),
        Arc::clone(&logging_obj),
        EndpointType::OpenAi,
    );
    call.url_route = "/v1/chat/completions".to_owned();
    drop(
        chunk_processor(call, ok_chunks(vec![b"alpha".to_vec()]))
            .collect::<Vec<_>>()
            .await,
    );
    let job = harness.queue.take_all().into_iter().next().unwrap();
    job.work.await;
    let routes = harness.route.calls();
    assert_eq!(routes[0].end_time, time("2024-01-01T00:00:07+00:00"));
    assert_eq!(routes[0].url_route, "/v1/chat/completions");
    assert!(
        model_details(logging_obj.as_ref())
            .get("completion_start_time")
            .is_some()
    );
}
