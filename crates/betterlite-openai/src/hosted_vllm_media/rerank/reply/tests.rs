#![cfg(test)]

use std::fs;
use std::path::PathBuf;

use rstest::rstest;
use serde_json::Value;
use uuid::Uuid;

use super::{transform_rerank_response, transform_response};
use crate::api::rerank_contracts::records::RerankResponse;
use crate::hosted_vllm_media::rerank::HostedVllmRerankError;

fn value(text: &str) -> Value {
    serde_json::from_str(text).expect("the case is JSON")
}

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
    let response = decode(&value(
        r#"{
            "id": "reply-1",
            "results": [
                {"index": 0, "relevance_score": 0.5, "document": {"text": "first"}},
                {"index": 1, "relevance_score": 0.25}
            ],
            "usage": {"total_tokens": 12}
        }"#,
    ));
    assert_eq!(
        encoded(&response),
        value(
            r#"{
                "id": "reply-1",
                "results": [
                    {"index": 0, "relevance_score": 0.5, "document": {"text": "first"}},
                    {"index": 1, "relevance_score": 0.25}
                ],
                "meta": {
                    "billed_units": {"total_tokens": 12},
                    "tokens": {"input_tokens": 12}
                }
            }"#,
        )
    );
}

#[rstest]
#[case(r#"{"results": []}"#)]
#[case(r#"{"id": "", "results": []}"#)]
fn generates_an_id_when_the_reply_has_none(#[case] text: &str) {
    assert_eq!(
        encoded(&decode(&value(text))).get("id"),
        Some(&Value::String(String::from(
            "00000000-0000-0000-0000-000000000007"
        )))
    );
}

#[rstest]
#[case(r#"{"results": [{"index": 0, "relevance_score": 0.5}]}"#)]
#[case(r#"{"results": [{"index": 0, "relevance_score": 0.5, "document": {}}]}"#)]
#[case(r#"{"results": [{"index": 0, "relevance_score": 0.5, "document": "first"}]}"#)]
fn drops_a_document_that_is_not_a_nonempty_object(#[case] text: &str) {
    let response = decode(&value(text));
    assert_eq!(encoded(&response).pointer("/results/0/document"), None);
}

#[test]
fn a_document_without_text_reads_as_an_empty_one() {
    let response = decode(&value(
        r#"{"results": [{"index": 0, "relevance_score": 0.5, "document": {"score": 1}}]}"#,
    ));
    assert_eq!(
        encoded(&response).pointer("/results/0/document"),
        Some(&value(r#"{"text": ""}"#))
    );
}

#[test]
fn a_document_text_that_is_not_a_string_reads_as_empty() {
    let response = decode(&value(
        r#"{"results": [{"index": 0, "relevance_score": 0.5, "document": {"text": 5}}]}"#,
    ));
    assert_eq!(
        encoded(&response).pointer("/results/0/document/text"),
        Some(&Value::String(String::new()))
    );
}

#[rstest]
#[case("{}")]
#[case(r#"{"results": null}"#)]
fn missing_results_is_an_error(#[case] text: &str) {
    let reply = value(text);
    let error = decode_error(&reply);
    let HostedVllmRerankError::NoResults { response } = error else {
        panic!("a reply without results must be a NoResults error");
    };
    assert_eq!(response, reply);
}

#[test]
fn results_that_are_not_an_array_are_a_shape_error() {
    assert!(matches!(
        decode_error(&value(r#"{"results": 5}"#)),
        HostedVllmRerankError::Shape(..)
    ));
}

#[rstest]
#[case(r#"{"results": [{"index": 0}]}"#)]
#[case(r#"{"results": [{"relevance_score": 0.5}]}"#)]
#[case(r#"{"results": [{"index": "0", "relevance_score": 0.5}]}"#)]
#[case(r#"{"results": [{"index": 0, "relevance_score": "0.5"}]}"#)]
fn a_result_without_its_index_or_exact_score_is_an_error(#[case] text: &str) {
    assert!(matches!(
        decode_error(&value(text)),
        HostedVllmRerankError::MissingResultFields { .. }
    ));
}

#[test]
fn a_score_the_exact_decimal_cannot_hold_is_an_error() {
    let reply = value(r#"{"results":[{"index":0,"relevance_score":1e400}]}"#);
    assert!(matches!(
        decode_error(&reply),
        HostedVllmRerankError::Score(..)
    ));
}

#[rstest]
#[case(r#"{"results": []}"#)]
#[case(r#"{"results": [], "usage": null}"#)]
fn an_absent_or_null_usage_reads_as_zero_tokens(#[case] text: &str) {
    assert_eq!(
        encoded(&decode(&value(text))).get("meta"),
        Some(&value(
            r#"{"billed_units": {"total_tokens": 0}, "tokens": {"input_tokens": 0}}"#,
        ))
    );
}

#[rstest]
#[case(r#"{"results": [], "usage": 5}"#)]
#[case(r#"{"results": [], "usage": "many"}"#)]
#[case(r#"{"results": [], "usage": [1]}"#)]
#[case(r#"{"results": [], "usage": true}"#)]
fn a_usage_that_is_not_an_object_is_a_shape_error(#[case] text: &str) {
    assert!(matches!(
        decode_error(&value(text)),
        HostedVllmRerankError::Shape(..)
    ));
}

#[test]
fn a_null_usage_count_stays_null() {
    assert_eq!(
        encoded(&decode(&value(
            r#"{"results": [], "usage": {"total_tokens": null}}"#,
        )))
        .get("meta"),
        Some(&value(
            r#"{"billed_units": {"total_tokens": null}, "tokens": {"input_tokens": null}}"#,
        ))
    );
}

#[rstest]
#[case(r#"{"results": [], "usage": {"total_tokens": "twelve"}}"#)]
#[case(r#"{"results": [], "usage": {"total_tokens": 1.5}}"#)]
fn a_usage_count_that_is_not_an_exact_number_is_a_shape_error(#[case] text: &str) {
    assert!(matches!(
        decode_error(&value(text)),
        HostedVllmRerankError::Shape(..)
    ));
}

#[test]
fn a_malformed_body_is_a_parse_error_with_its_status() {
    let error = transform_rerank_response(500, b"not json", &generated)
        .expect_err("the body must not decode");
    let HostedVllmRerankError::Parse { text, status_code } = error else {
        panic!("a malformed body must be a parse error");
    };
    assert_eq!(text, "not json");
    assert_eq!(status_code, 500);
}

#[test]
fn a_body_that_is_not_utf8_reports_the_refused_text() {
    let error = transform_rerank_response(200, &[0xff, 0xfe], &generated)
        .expect_err("the body must not decode");
    let HostedVllmRerankError::Parse { text, status_code } = error else {
        panic!("a body that is not UTF-8 must be a parse error");
    };
    assert_eq!(text, "<body is not UTF-8>");
    assert_eq!(status_code, 200);
}

#[test]
fn a_body_that_is_json_but_not_an_object_has_no_results() {
    // serde reads an empty sequence as a struct whose defaulted fields are absent, so no results key reaches the transform
    assert!(matches!(
        transform_rerank_response(200, b"[]", &generated),
        Err(HostedVllmRerankError::NoResults { .. })
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
        Some(&Value::from(0_i32))
    );
}

#[rstest]
fn replays_the_hosted_vllm_rerank_reply_corpus(
    #[files("fuzz/corpus/hosted_vllm_rerank_reply/*")] path: PathBuf,
) {
    let bytes = fs::read(path).expect("corpus file");
    let _reply = transform_rerank_response(200, &bytes, &generated);
}
