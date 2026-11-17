#![cfg(test)]

use std::fs;
use std::path::PathBuf;

use rstest::rstest;
use serde_json::{Value, json};
use uuid::Uuid;

use super::{transform_rerank_response, transform_response};
use crate::api::rerank_contracts::records::RerankResponse;
use crate::hosted_vllm_media::rerank::HostedVllmRerankError;

fn generated() -> Uuid {
    Uuid::from_u128(7)
}

fn decode(value: &Value) -> RerankResponse {
    transform_response(value, &generated).expect("the reply decodes")
}

fn encoded(response: &RerankResponse) -> Value {
    serde_json::to_value(response).expect("the response serializes")
}

fn decode_error(value: &Value) -> HostedVllmRerankError {
    transform_response(value, &generated).expect_err("the reply must not decode")
}

#[test]
fn transforms_a_full_reply() {
    let response = decode(&json!({
        "id": "reply-1",
        "results": [
            {"index": 0, "relevance_score": 0.5, "document": {"text": "first"}},
            {"index": 1, "relevance_score": 0.25},
        ],
        "usage": {"total_tokens": 12},
    }));
    assert_eq!(
        encoded(&response),
        json!({
            "id": "reply-1",
            "results": [
                {"index": 0, "relevance_score": 0.5, "document": {"text": "first"}},
                {"index": 1, "relevance_score": 0.25},
            ],
            "meta": {
                "billed_units": {"total_tokens": 12},
                "tokens": {"input_tokens": 12},
            },
        })
    );
}

#[rstest]
#[case(json!({"results": []}))]
#[case(json!({"id": "", "results": []}))]
fn generates_an_id_when_the_reply_has_none(#[case] reply: Value) {
    assert_eq!(
        encoded(&decode(&reply)).get("id"),
        Some(&json!("00000000-0000-0000-0000-000000000007"))
    );
}

#[rstest]
#[case(json!({"index": 0, "relevance_score": 0.5}))]
#[case(json!({"index": 0, "relevance_score": 0.5, "document": {}}))]
#[case(json!({"index": 0, "relevance_score": 0.5, "document": "first"}))]
fn drops_a_document_that_is_not_a_nonempty_object(#[case] result: Value) {
    let response = decode(&json!({"results": [result]}));
    assert_eq!(encoded(&response).pointer("/results/0/document"), None);
}

#[test]
fn a_document_without_text_reads_as_an_empty_one() {
    let response = decode(&json!({
        "results": [{"index": 0, "relevance_score": 0.5, "document": {"score": 1}}],
    }));
    assert_eq!(
        encoded(&response).pointer("/results/0/document"),
        Some(&json!({"text": ""}))
    );
}

#[test]
fn a_document_text_that_is_not_a_string_reads_as_empty() {
    let response = decode(&json!({
        "results": [{"index": 0, "relevance_score": 0.5, "document": {"text": 5}}],
    }));
    assert_eq!(
        encoded(&response).pointer("/results/0/document/text"),
        Some(&json!(""))
    );
}

#[rstest]
#[case(json!({}))]
#[case(json!({"results": null}))]
fn missing_results_is_an_error(#[case] reply: Value) {
    let error = decode_error(&reply);
    match error {
        HostedVllmRerankError::NoResults { response } => assert_eq!(response, reply),
        other => panic!("unexpected error {other}"),
    }
}

#[test]
fn results_that_are_not_an_array_are_a_shape_error() {
    assert!(matches!(
        decode_error(&json!({"results": 5})),
        HostedVllmRerankError::Shape { .. }
    ));
}

#[rstest]
#[case(json!({"results": [{"index": 0}]}))]
#[case(json!({"results": [{"relevance_score": 0.5}]}))]
#[case(json!({"results": [{"index": "0", "relevance_score": 0.5}]}))]
#[case(json!({"results": [{"index": 0, "relevance_score": "0.5"}]}))]
fn a_result_without_its_index_or_exact_score_is_an_error(#[case] reply: Value) {
    assert!(matches!(
        decode_error(&reply),
        HostedVllmRerankError::MissingResultFields { .. }
    ));
}

#[test]
fn a_score_the_exact_decimal_cannot_hold_is_an_error() {
    let reply: Value = serde_json::from_str(r#"{"results":[{"index":0,"relevance_score":1e400}]}"#)
        .expect("the fixture is valid JSON");
    assert!(matches!(
        decode_error(&reply),
        HostedVllmRerankError::Score { .. }
    ));
}

#[test]
fn an_absent_usage_reads_as_zero_tokens() {
    assert_eq!(
        encoded(&decode(&json!({"results": []}))).get("meta"),
        Some(&json!({
            "billed_units": {"total_tokens": 0},
            "tokens": {"input_tokens": 0},
        }))
    );
}

#[test]
fn a_null_usage_count_stays_null() {
    assert_eq!(
        encoded(&decode(
            &json!({"results": [], "usage": {"total_tokens": null}})
        ))
        .get("meta"),
        Some(&json!({
            "billed_units": {"total_tokens": null},
            "tokens": {"input_tokens": null},
        }))
    );
}

#[rstest]
#[case(json!({"results": [], "usage": {"total_tokens": "twelve"}}))]
#[case(json!({"results": [], "usage": {"total_tokens": 1.5}}))]
fn a_usage_count_that_is_not_an_exact_number_is_a_shape_error(#[case] reply: Value) {
    assert!(matches!(
        decode_error(&reply),
        HostedVllmRerankError::Shape { .. }
    ));
}

#[test]
fn a_malformed_body_is_a_parse_error_with_its_status() {
    let error = transform_rerank_response(500, b"not json", &generated)
        .expect_err("the body must not decode");
    match error {
        HostedVllmRerankError::Parse { text, status_code } => {
            assert_eq!(text, "not json");
            assert_eq!(status_code, 500);
        }
        other => panic!("unexpected error {other}"),
    }
}

#[test]
fn a_body_that_is_json_but_not_an_object_is_a_shape_error() {
    assert!(matches!(
        transform_rerank_response(200, b"[]", &generated),
        Err(HostedVllmRerankError::Shape { .. })
    ));
}

#[test]
fn a_valid_body_decodes_through_the_byte_entry_point() {
    let response = transform_rerank_response(
        200,
        br#"{"results":[{"index":0,"relevance_score":0.5}]}"#,
        &generated,
    )
    .expect("the body decodes");
    assert_eq!(
        encoded(&response).pointer("/results/0/index"),
        Some(&json!(0))
    );
}

#[rstest]
fn replays_the_hosted_vllm_rerank_reply_corpus(
    #[files("fuzz/corpus/hosted_vllm_rerank_reply/*")] path: PathBuf,
) {
    let bytes = fs::read(path).expect("corpus file");
    let _reply = transform_rerank_response(200, &bytes, &generated);
}
