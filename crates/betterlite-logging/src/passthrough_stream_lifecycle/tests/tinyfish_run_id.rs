#![cfg(test)]

use futures_util::StreamExt as _;
use indexmap::IndexMap;

use crate::passthrough_stream_lifecycle::chunk_processor::chunk_processor;
use crate::passthrough_stream_lifecycle::ports::StreamJobKind;
use crate::passthrough_stream_lifecycle::support::{
    Harness, object, ok_chunks, processor_call, time,
};
use crate::passthrough_stream_logging::types::EndpointType;

async fn run(harness: &Harness, endpoint_type: EndpointType, chunks: Vec<Vec<u8>>) {
    let call = processor_call(
        harness.lifecycle(false),
        object(&[], IndexMap::new()),
        endpoint_type,
    );
    drop(
        chunk_processor(call, ok_chunks(chunks))
            .collect::<Vec<_>>()
            .await,
    );
}

#[tokio::test]
async fn a_run_id_in_a_complete_frame_starts_the_billing() {
    let harness = Harness::new(&["2024-01-01T00:00:05+00:00"]).with_tinyfish(false, Some("run-1"));
    run(
        &harness,
        EndpointType::Tinyfish,
        vec![b"data: {\"run_id\":\"run-1\"}\n\n".to_vec()],
    )
    .await;
    assert_eq!(
        harness.tinyfish.started(),
        vec![("run-1".to_owned(), time("2024-01-01T00:00:00+00:00"))]
    );
    assert_eq!(
        harness.tinyfish.scans.lock().unwrap().clone(),
        vec![b"data: {\"run_id\":\"run-1\"}\n\n".to_vec()]
    );
}

#[tokio::test]
async fn the_scan_halts_after_the_run_id_is_found() {
    let harness = Harness::new(&["2024-01-01T00:00:01+00:00"]).with_tinyfish(false, Some("run-1"));
    run(
        &harness,
        EndpointType::Tinyfish,
        vec![
            b"data: {\"run_id\":\"run-1\"}\n\n".to_vec(),
            b"data: {\"run_id\":\"run-2\"}\n\n".to_vec(),
        ],
    )
    .await;
    assert_eq!(harness.tinyfish.scans.lock().unwrap().len(), 1);
    assert_eq!(harness.tinyfish.started().len(), 1);
}

#[tokio::test]
async fn an_incomplete_frame_is_never_scanned() {
    let harness = Harness::new(&["2024-01-01T00:00:01+00:00"]).with_tinyfish(false, Some("run-1"));
    run(
        &harness,
        EndpointType::Tinyfish,
        vec![b"data: {\"run_id\":\"run-1\"}".to_vec()],
    )
    .await;
    assert!(harness.tinyfish.scans.lock().unwrap().is_empty());
    assert!(harness.tinyfish.started().is_empty());
}

#[tokio::test]
async fn a_complete_frame_without_a_run_id_is_not_parsed() {
    let harness = Harness::new(&["2024-01-01T00:00:01+00:00"]).with_tinyfish(false, Some("run-1"));
    run(
        &harness,
        EndpointType::Tinyfish,
        vec![b"data: {\"value\":1}\n\n".to_vec()],
    )
    .await;
    assert!(harness.tinyfish.scans.lock().unwrap().is_empty());
    assert!(harness.tinyfish.started().is_empty());
}

#[tokio::test]
async fn the_scan_continues_until_the_run_id_appears() {
    let harness = Harness::new(&["2024-01-01T00:00:01+00:00"]).with_tinyfish(false, Some("run-1"));
    run(
        &harness,
        EndpointType::Tinyfish,
        vec![
            b"data: {\"value\":1}\n\n".to_vec(),
            b"data: {\"run_id\":\"run-1\"}\n\n".to_vec(),
        ],
    )
    .await;
    assert_eq!(
        harness.tinyfish.scans.lock().unwrap().clone(),
        vec![b"data: {\"run_id\":\"run-1\"}\n\n".to_vec()]
    );
    assert_eq!(harness.tinyfish.started().len(), 1);
}

#[tokio::test]
async fn a_run_id_split_across_chunks_is_found_once_reassembled() {
    let harness = Harness::new(&["2024-01-01T00:00:01+00:00"]).with_tinyfish(false, Some("run-1"));
    run(
        &harness,
        EndpointType::Tinyfish,
        vec![b"data: {\"run_".to_vec(), b"id\":\"run-1\"}\n\n".to_vec()],
    )
    .await;
    assert_eq!(
        harness.tinyfish.scans.lock().unwrap().clone(),
        vec![b"data: {\"run_id\":\"run-1\"}\n\n".to_vec()]
    );
    assert_eq!(harness.tinyfish.started().len(), 1);
}

#[tokio::test]
async fn the_whole_stream_still_reaches_the_logging_route() {
    let harness = Harness::new(&["2024-01-01T00:00:01+00:00"]).with_tinyfish(false, Some("run-1"));
    let chunks = vec![
        b"data: {\"value\":1}\n\n".to_vec(),
        b"data: {\"run_id\":\"run-1\"}\n\n".to_vec(),
    ];
    run(&harness, EndpointType::Tinyfish, chunks.clone()).await;
    let job = harness.queue.take_all().into_iter().next().unwrap();
    job.work.await;
    assert_eq!(harness.route.calls()[0].raw_bytes, chunks);
}

#[tokio::test]
async fn a_non_tinyfish_endpoint_never_scans() {
    let harness = Harness::new(&["2024-01-01T00:00:01+00:00"]).with_tinyfish(false, Some("run-1"));
    run(
        &harness,
        EndpointType::OpenAi,
        vec![b"data: {\"run_id\":\"run-1\"}\n\n".to_vec()],
    )
    .await;
    assert!(harness.tinyfish.scans.lock().unwrap().is_empty());
    assert!(harness.tinyfish.started().is_empty());
    assert_eq!(harness.queue.kinds(), vec![StreamJobKind::Success]);
}
