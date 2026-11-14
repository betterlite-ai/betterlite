#![cfg(test)]

use std::fs;
use std::path::PathBuf;

use rstest::rstest;

use super::{VERTEX_BATCH_PAYLOAD_LIMIT, VertexEndpointResponse, decode_vertex_batch_payload};
use crate::vertex_batch_create::error::VertexBatchCreateError;
use crate::vertex_batch_records::types::VertexBatchPredictionResponse;

const FRAME: &str = r#"{"name":""}"#;

fn job(bytes: &[u8]) -> Result<VertexBatchPredictionResponse, VertexBatchCreateError> {
    decode_vertex_batch_payload(bytes)
}

fn endpoint(bytes: &[u8]) -> Result<VertexEndpointResponse, VertexBatchCreateError> {
    decode_vertex_batch_payload(bytes)
}

// a job whose name pads the document to exactly this many bytes
fn padded_job(total: usize) -> Vec<u8> {
    format!(r#"{{"name":"{}"}}"#, "a".repeat(total - FRAME.len())).into_bytes()
}

#[rstest]
fn replays_the_vertex_batch_payload_corpus(
    #[files("fuzz/corpus/vertex_batch_payload/*")] path: PathBuf,
) {
    let bytes = fs::read(path).unwrap();
    drop(job(&bytes));
    drop(endpoint(&bytes));
}

#[test]
fn the_payload_limit_is_inclusive() {
    let at_limit = job(&padded_job(VERTEX_BATCH_PAYLOAD_LIMIT)).unwrap();
    assert_eq!(
        at_limit.name.unwrap().len(),
        VERTEX_BATCH_PAYLOAD_LIMIT - FRAME.len()
    );
    let error = job(&padded_job(VERTEX_BATCH_PAYLOAD_LIMIT + 1)).unwrap_err();
    assert!(matches!(
        error,
        VertexBatchCreateError::PayloadTooLarge {
            limit: VERTEX_BATCH_PAYLOAD_LIMIT
        }
    ));
}

#[rstest]
#[case::array(b"[]".as_slice())]
#[case::number(b"1")]
#[case::null(b"null")]
#[case::string(br#""text""#)]
#[case::empty(b"")]
#[case::spaces_only(b"   ")]
fn a_root_that_is_not_an_object_is_refused(#[case] bytes: &[u8]) {
    for error in [job(bytes).unwrap_err(), endpoint(bytes).unwrap_err()] {
        assert!(error.to_string().contains("the document root"));
        assert_eq!(error.status_code(), 500);
    }
}

#[test]
fn surrounding_whitespace_is_accepted() {
    let parsed = job(b"\n\t {\"name\": \"x\"} \r\n").unwrap();
    assert_eq!(parsed.name.as_deref(), Some("x"));
}

#[test]
fn bytes_after_the_object_are_refused() {
    let error = job(br#"{"name":"x"} {"name":"y"}"#).unwrap_err();
    assert!(error.to_string().contains("the document root"));
}

#[test]
fn a_wrong_typed_field_names_its_path() {
    let error = endpoint(br#"{"deployedModels":[{"model":"ok"},{"model":7}]}"#).unwrap_err();
    assert!(error.to_string().contains("deployedModels[1].model"));
}

#[rstest]
#[case::absent(r"{}", None)]
#[case::null(r#"{"deployedModels":null}"#, None)]
#[case::empty(r#"{"deployedModels":[]}"#, Some(0))]
#[case::two(r#"{"deployedModels":[{"model":"a"},{"model":"b"}]}"#, Some(2))]
fn the_deployed_model_list_defaults_when_absent_or_null(
    #[case] body: &str,
    #[case] count: Option<usize>,
) {
    let parsed = endpoint(body.as_bytes()).unwrap();
    assert_eq!(parsed.deployed_models.map(|models| models.len()), count);
}

#[test]
fn a_null_model_reads_as_absent() {
    let parsed = endpoint(br#"{"deployedModels":[{"model":null},{}]}"#).unwrap();
    let models = parsed.deployed_models.unwrap();
    assert_eq!(models[0].model, None);
    assert_eq!(models[1].model, None);
}

#[test]
fn an_explicit_null_job_state_stays_distinct_from_an_absent_one() {
    assert_eq!(job(br#"{"state":null}"#).unwrap().state, Some(None));
    assert_eq!(job(b"{}").unwrap().state, None);
}
