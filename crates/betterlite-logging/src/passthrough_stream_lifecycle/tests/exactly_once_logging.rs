#![cfg(test)]

use std::sync::Arc;

use betterlite_models::decisions::Json;
use futures_util::StreamExt as _;
use indexmap::IndexMap;

use crate::passthrough_stream_lifecycle::chunk_processor::chunk_processor;
use crate::passthrough_stream_lifecycle::ports::StreamJobKind;
use crate::passthrough_stream_lifecycle::support::{
    Harness, json, litellm_params, object, ok_chunks, processor_call, reservation_params,
};
use crate::passthrough_stream_logging::types::EndpointType;

#[tokio::test]
async fn an_armed_deferred_stream_parks_its_logging() {
    let harness = Harness::new(&["2024-01-01T00:00:01+00:00"]).with_deferred();
    let lifecycle = harness.lifecycle(false);
    let logging_obj = object(&[], reservation_params());
    let call = processor_call(
        Arc::clone(&lifecycle),
        Arc::clone(&logging_obj),
        EndpointType::OpenAi,
    );
    drop(
        chunk_processor(call, ok_chunks(vec![b"alpha".to_vec()]))
            .collect::<Vec<_>>()
            .await,
    );
    assert!(harness.queue.kinds().is_empty());
    assert_eq!(
        harness.deferred_handle().kinds(),
        vec![StreamJobKind::Success]
    );
    assert_eq!(harness.binder.calls(), 1);
    let params = litellm_params(logging_obj.as_ref());
    let Some(Json::Object(metadata)) = params.get("metadata") else {
        panic!("the reservation metadata is present");
    };
    let Some(Json::Object(reservation)) = metadata.get("user_api_key_budget_reservation") else {
        panic!("the reservation is present");
    };
    assert_eq!(reservation.get("callback_bound"), Some(&json("true")));
}

#[tokio::test]
async fn the_parked_job_carries_the_stream_bytes() {
    let harness = Harness::new(&["2024-01-01T00:00:01+00:00"]).with_deferred();
    let lifecycle = harness.lifecycle(false);
    let chunks = vec![b"alpha".to_vec(), b"beta".to_vec()];
    let call = processor_call(
        Arc::clone(&lifecycle),
        object(&[], reservation_params()),
        EndpointType::OpenAi,
    );
    drop(
        chunk_processor(call, ok_chunks(chunks.clone()))
            .collect::<Vec<_>>()
            .await,
    );
    let job = harness
        .deferred_handle()
        .take_all()
        .into_iter()
        .next()
        .unwrap();
    assert!(harness.route.calls().is_empty());
    job.work.await;
    assert_eq!(harness.route.calls()[0].raw_bytes, chunks);
    assert!(harness.queue.kinds().is_empty());
}

#[tokio::test]
async fn without_deferred_logging_the_job_is_enqueued() {
    let harness = Harness::new(&["2024-01-01T00:00:01+00:00"]);
    let lifecycle = harness.lifecycle(false);
    let logging_obj = object(&[], reservation_params());
    let call = processor_call(
        Arc::clone(&lifecycle),
        Arc::clone(&logging_obj),
        EndpointType::OpenAi,
    );
    drop(
        chunk_processor(call, ok_chunks(vec![b"alpha".to_vec()]))
            .collect::<Vec<_>>()
            .await,
    );
    assert_eq!(harness.queue.kinds(), vec![StreamJobKind::Success]);
    assert_eq!(harness.binder.calls(), 1);
    assert!(
        harness
            .deferred
            .as_ref()
            .is_none_or(|deferred| deferred.kinds().is_empty())
    );
}

#[tokio::test]
async fn an_armed_deferred_stream_with_no_bytes_logs_nothing() {
    let harness = Harness::new(&["2024-01-01T00:00:01+00:00"]).with_deferred();
    let lifecycle = harness.lifecycle(false);
    let call = processor_call(
        Arc::clone(&lifecycle),
        object(&[], reservation_params()),
        EndpointType::OpenAi,
    );
    let collected: Vec<_> = chunk_processor(call, ok_chunks(Vec::new())).collect().await;
    assert!(collected.is_empty());
    assert!(harness.deferred_handle().kinds().is_empty());
    assert!(harness.queue.kinds().is_empty());
    assert_eq!(harness.binder.calls(), 0);
}

#[tokio::test]
async fn an_armed_deferred_stream_with_status_400_logs_nothing() {
    let harness = Harness::new(&["2024-01-01T00:00:01+00:00"]).with_deferred();
    let lifecycle = harness.lifecycle(false);
    let mut call = processor_call(
        Arc::clone(&lifecycle),
        object(&[], reservation_params()),
        EndpointType::OpenAi,
    );
    call.status_code = 400;
    drop(
        chunk_processor(call, ok_chunks(vec![b"alpha".to_vec()]))
            .collect::<Vec<_>>()
            .await,
    );
    assert!(harness.deferred_handle().kinds().is_empty());
    assert!(harness.queue.kinds().is_empty());
    assert_eq!(harness.binder.calls(), 0);
}

#[tokio::test]
async fn a_clean_completion_creates_exactly_one_job() {
    let harness = Harness::new(&["2024-01-01T00:00:01+00:00"]);
    let lifecycle = harness.lifecycle(false);
    let call = processor_call(
        Arc::clone(&lifecycle),
        object(&[], reservation_params()),
        EndpointType::OpenAi,
    );
    drop(
        chunk_processor(call, ok_chunks(vec![b"alpha".to_vec()]))
            .collect::<Vec<_>>()
            .await,
    );
    assert_eq!(harness.queue.kinds().len(), 1);
    assert_eq!(harness.binder.calls(), 1);
    assert!(harness.failure_dispatch.failures().is_empty());
    assert!(harness.route.calls().is_empty());
}

#[tokio::test]
async fn the_logged_request_body_is_never_null() {
    let harness = Harness::new(&["2024-01-01T00:00:01+00:00"]);
    let lifecycle = harness.lifecycle(false);
    let call = processor_call(
        Arc::clone(&lifecycle),
        object(&[], IndexMap::new()),
        EndpointType::OpenAi,
    );
    drop(
        chunk_processor(call, ok_chunks(vec![b"alpha".to_vec()]))
            .collect::<Vec<_>>()
            .await,
    );
    let job = harness.queue.take_all().into_iter().next().unwrap();
    assert!(harness.route.calls().is_empty());
    job.work.await;
    assert_eq!(harness.route.calls()[0].request_body, Some(json("{}")));
}
