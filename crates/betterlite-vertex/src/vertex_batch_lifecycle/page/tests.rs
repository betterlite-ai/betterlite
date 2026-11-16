#![cfg(test)]

use std::fs;
use std::path::PathBuf;

use betterlite_http::proxy::UpstreamResponse;
use http::header::HeaderMap;
use rstest::rstest;

use super::decode_vertex_batch_page;
use crate::vertex_batch_create::error::VertexBatchCreateError;
use crate::vertex_batch_create::payload::VERTEX_BATCH_PAYLOAD_LIMIT;
use crate::vertex_batch_lifecycle::error::VertexBatchLifecycleError;

fn reply(status: u16, body: &[u8]) -> UpstreamResponse {
    UpstreamResponse {
        status,
        headers: HeaderMap::new(),
        body: body.to_vec(),
    }
}

#[rstest]
fn replays_the_vertex_batch_page_corpus(#[files("fuzz/corpus/vertex_batch_page/*")] path: PathBuf) {
    let bytes = fs::read(path).unwrap();
    drop(decode_vertex_batch_page(&reply(200, &bytes)));
}

#[test]
fn jobs_and_the_next_token_are_read_from_a_200_page() {
    let page = decode_vertex_batch_page(&reply(
        200,
        br#"{"batchPredictionJobs":[{"name":"projects/p/jobs/1"}],"nextPageToken":"t2"}"#,
    ))
    .unwrap();
    let jobs = page.batch_prediction_jobs.unwrap();
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].name.as_deref(), Some("projects/p/jobs/1"));
    assert_eq!(page.next_page_token.as_deref(), Some("t2"));
}

#[test]
fn a_null_job_list_and_an_empty_token_are_kept_as_sent() {
    let page = decode_vertex_batch_page(&reply(
        200,
        br#"{"batchPredictionJobs":null,"nextPageToken":""}"#,
    ))
    .unwrap();
    assert!(page.batch_prediction_jobs.is_none());
    assert_eq!(page.next_page_token.as_deref(), Some(""));
}

#[rstest]
#[case::created(201)]
#[case::bad_request(400)]
#[case::server_error(500)]
fn a_non_200_page_is_refused_with_its_status(#[case] status: u16) {
    let error = decode_vertex_batch_page(&reply(status, b"no page here")).unwrap_err();
    assert!(matches!(
        error,
        VertexBatchLifecycleError::ListStatus { .. }
    ));
    assert_eq!(error.status_code(), status);
    assert!(error.to_string().contains("no page here"));
}

#[test]
fn a_refused_page_keeps_the_text_before_its_first_invalid_byte() {
    let error = decode_vertex_batch_page(&reply(500, b"page \xff gone")).unwrap_err();
    assert!(error.to_string().ends_with(": page "));
}

#[test]
fn a_malformed_page_names_its_field() {
    let error =
        decode_vertex_batch_page(&reply(200, br#"{"batchPredictionJobs":"nope"}"#)).unwrap_err();
    assert!(matches!(
        error,
        VertexBatchLifecycleError::Batch(VertexBatchCreateError::Decode { .. })
    ));
    assert_eq!(error.status_code(), 500);
    assert!(error.to_string().contains("batchPredictionJobs"));
    let nested = decode_vertex_batch_page(&reply(200, br#"{"batchPredictionJobs":[{"state":7}]}"#))
        .unwrap_err();
    assert!(nested.to_string().contains("batchPredictionJobs[0].state"));
}

#[test]
fn a_root_that_is_not_an_object_is_refused() {
    let error = decode_vertex_batch_page(&reply(200, b"[]")).unwrap_err();
    assert!(error.to_string().contains("the document root"));
    assert_eq!(error.status_code(), 500);
}

#[test]
fn a_page_body_takes_the_batch_payload_bound() {
    let oversized = format!(
        r#"{{"nextPageToken":"{}"}}"#,
        "a".repeat(VERTEX_BATCH_PAYLOAD_LIMIT)
    );
    let error = decode_vertex_batch_page(&reply(200, oversized.as_bytes())).unwrap_err();
    assert!(matches!(
        error,
        VertexBatchLifecycleError::Batch(VertexBatchCreateError::PayloadTooLarge {
            limit: VERTEX_BATCH_PAYLOAD_LIMIT
        })
    ));
    assert_eq!(error.status_code(), 500);
}
