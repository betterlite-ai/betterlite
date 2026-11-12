#![cfg(test)]

use std::fs;
use std::path::Path;

use betterlite_models::decisions::Json;
use rstest::rstest;
use serde_json::Value;

use super::records::{RerankDocument, RerankRequest, RerankResponse, RerankResponseMeta};
use super::wire::{RerankContract, RerankContractError, RerankRecordKind, decode_rerank_contracts};
use crate::api::responses_shapes::wire::WireDecodeLimits;
use crate::api::vector_store_records::records::score::Score;

fn limits() -> WireDecodeLimits {
    WireDecodeLimits {
        max_json_bytes: 0x0010_0000,
        max_event_bytes: 0x0004_0000,
        max_items: 10_000,
    }
}

fn decode(bytes: &[u8], kind: RerankRecordKind) -> Result<RerankContract, RerankContractError> {
    decode_rerank_contracts(bytes, kind, limits())
}

fn request(bytes: &[u8]) -> RerankRequest {
    match decode(bytes, RerankRecordKind::Request).unwrap() {
        RerankContract::Request(record) => record,
        RerankContract::Response(_) => panic!("the decoder returned a response"),
    }
}

fn response(bytes: &[u8]) -> RerankResponse {
    match decode(bytes, RerankRecordKind::Response).unwrap() {
        RerankContract::Response(record) => record,
        RerankContract::Request(_) => panic!("the decoder returned a request"),
    }
}

fn json_value(source: &str) -> Value {
    serde_json::from_str(source).unwrap()
}

#[test]
fn rerank_wire_records_keep_documents_and_default_the_optionals() {
    let decoded = request(
        br#"{"model":"m","query":"q","documents":["text",{"title":"t","body":"b"}],"extra":"ignored"}"#,
    );
    assert_eq!(
        decoded.documents,
        vec![
            RerankDocument::Text(String::from("text")),
            RerankDocument::Object(
                [("title", "t"), ("body", "b")]
                    .into_iter()
                    .map(|(name, value)| (name.to_owned(), Json::Text(value.to_owned())))
                    .collect()
            ),
        ]
    );
    assert_eq!(
        serde_json::to_value(&decoded).unwrap(),
        json_value(
            r#"{"model":"m","query":"q","top_n":null,"documents":["text",{"title":"t","body":"b"}],"rank_fields":null,"return_documents":null,"max_chunks_per_doc":null,"max_tokens_per_doc":null,"instruction":null,"truncate_prompt_tokens":null,"truncation_side":null,"max_tokens_per_query":null}"#
        )
    );
}

#[rstest]
#[case::empty_documents(br#"{"model":"m","query":"q","documents":[]}"#)]
#[case::zero_top_n(br#"{"model":"m","query":"q","documents":[],"top_n":0}"#)]
#[case::left(br#"{"model":"m","query":"q","documents":[],"truncation_side":"left"}"#)]
#[case::right(br#"{"model":"m","query":"q","documents":[],"truncation_side":"right"}"#)]
#[case::null_side(br#"{"model":"m","query":"q","documents":[],"truncation_side":null}"#)]
#[case::zero_truncation(
    br#"{"model":"m","query":"q","documents":[],"truncate_prompt_tokens":0,"max_tokens_per_query":0,"max_chunks_per_doc":0,"max_tokens_per_doc":0,"instruction":"","return_documents":false}"#
)]
fn request_accepts_declared_values(#[case] input: &[u8]) {
    assert!(matches!(
        decode(input, RerankRecordKind::Request),
        Ok(RerankContract::Request(_))
    ));
}

#[rstest]
#[case::missing_model(br#"{"query":"q","documents":[]}"#)]
#[case::missing_query(br#"{"model":"m","documents":[]}"#)]
#[case::missing_documents(br#"{"model":"m","query":"q"}"#)]
#[case::middle_side(br#"{"model":"m","query":"q","documents":[],"truncation_side":"middle"}"#)]
#[case::document_number(br#"{"model":"m","query":"q","documents":[7]}"#)]
#[case::model_number(br#"{"model":7,"query":"q","documents":[]}"#)]
fn request_rejects_missing_or_mistyped_fields(#[case] input: &[u8]) {
    let _error = decode(input, RerankRecordKind::Request).unwrap_err();
}

#[test]
fn response_defaults_every_field_to_none() {
    let decoded = response(b"{}");
    assert_eq!(decoded.id, None);
    assert_eq!(decoded.results, None);
    assert_eq!(decoded.meta, None);
    assert_eq!(
        serde_json::to_value(&decoded).unwrap(),
        json_value(r#"{"id":null,"results":null,"meta":null}"#)
    );
}

#[test]
fn response_retains_result_order_index_score_and_document() {
    let decoded = response(
        br#"{"id":"r","results":[{"index":2,"relevance_score":0.125,"document":{"text":"t"}},{"index":0,"relevance_score":-0.5}],"meta":{"api_version":{"version":"2"},"billed_units":{"search_units":3,"total_tokens":9},"tokens":{"input_tokens":1,"output_tokens":2}}}"#,
    );
    assert_eq!(
        serde_json::to_value(&decoded).unwrap(),
        json_value(
            r#"{"id":"r","results":[{"index":2,"relevance_score":0.125,"document":{"text":"t"}},{"index":0,"relevance_score":-0.5}],"meta":{"api_version":{"version":"2"},"billed_units":{"search_units":3,"total_tokens":9},"tokens":{"input_tokens":1,"output_tokens":2}}}"#
        )
    );
}

#[rstest]
#[case::missing_index(br#"{"results":[{"relevance_score":0.5}]}"#)]
#[case::missing_score(br#"{"results":[{"index":0}]}"#)]
#[case::null_index(br#"{"results":[{"index":null,"relevance_score":0.5}]}"#)]
#[case::null_score(br#"{"results":[{"index":0,"relevance_score":null}]}"#)]
#[case::null_document(br#"{"results":[{"index":0,"relevance_score":0.5,"document":null}]}"#)]
#[case::document_without_text(br#"{"results":[{"index":0,"relevance_score":0.5,"document":{}}]}"#)]
#[case::id_number(br#"{"id":7}"#)]
#[case::results_object(br#"{"results":{}}"#)]
fn response_rejects_missing_or_mistyped_fields(#[case] input: &[u8]) {
    let _error = decode(input, RerankRecordKind::Response).unwrap_err();
}

#[test]
fn a_null_document_is_refused_and_an_absent_one_is_kept_absent() {
    assert!(matches!(
        decode(
            br#"{"results":[{"index":0,"relevance_score":0.5,"document":null}]}"#,
            RerankRecordKind::Response
        ),
        Err(RerankContractError::NullDocument)
    ));
    let absent = response(br#"{"results":[{"index":0,"relevance_score":0.5}]}"#);
    assert_eq!(absent.results.unwrap()[0].document, None);
}

#[rstest]
#[case::meta_null_members(
    br#"{"meta":{"api_version":null,"billed_units":null,"tokens":null}}"#,
    r#"{"api_version":null,"billed_units":null,"tokens":null}"#
)]
#[case::absent_members(br#"{"meta":{}}"#, "{}")]
#[case::empty_units(
    br#"{"meta":{"billed_units":{},"tokens":{"input_tokens":null}}}"#,
    r#"{"billed_units":{},"tokens":{"input_tokens":null}}"#
)]
fn response_meta_keeps_absent_and_null_apart(#[case] input: &[u8], #[case] expected: &str) {
    let decoded = response(input);
    let meta: &RerankResponseMeta = decoded.meta.as_ref().unwrap();
    assert_eq!(serde_json::to_value(meta).unwrap(), json_value(expected));
}

#[test]
fn response_ignores_unknown_fields_and_never_writes_hidden_params() {
    let mut decoded = response(br#"{"id":"r","top_extra":5}"#);
    decoded
        .hidden_params
        .insert("model_id".to_owned(), Json::Text(String::from("secret")));
    assert_eq!(
        serde_json::to_value(&decoded).unwrap(),
        json_value(r#"{"id":"r","results":null,"meta":null}"#)
    );
}

#[test]
fn response_accessors_follow_the_python_object() {
    let decoded = response(b"{}");
    assert_eq!(decoded.field("id"), Some(Json::Null));
    assert_eq!(decoded.field("results"), Some(Json::Null));
    assert_eq!(decoded.field("meta"), Some(Json::Null));
    assert_eq!(decoded.field("hidden_params"), None);
    assert_eq!(decoded.field("missing"), None);
    assert_eq!(decoded.item("id").unwrap(), Json::Null);
    let error = decoded.item("missing").unwrap_err();
    assert_eq!(error.name, "missing");
    assert_eq!(
        error.to_string(),
        "the rerank response has no field `missing`"
    );
}

#[test]
fn accessors_expose_decoded_results() {
    let decoded = response(
        br#"{"id":"r","results":[{"index":2,"relevance_score":0.125,"document":{"text":"t"}}]}"#,
    );
    assert_eq!(
        serde_json::to_value(decoded.item("results").unwrap()).unwrap(),
        json_value(r#"[{"index":2,"relevance_score":0.125,"document":{"text":"t"}}]"#)
    );
    assert_eq!(decoded.field("id"), Some(Json::Text(String::from("r"))));
}

#[test]
fn unknown_record_kind_is_refused() {
    assert!(matches!(
        RerankRecordKind::try_from(2_u8),
        Err(RerankContractError::UnknownKind(2))
    ));
    assert!(matches!(
        RerankRecordKind::try_from(0_u8),
        Ok(RerankRecordKind::Request)
    ));
    assert!(matches!(
        RerankRecordKind::try_from(1_u8),
        Ok(RerankRecordKind::Response)
    ));
}

#[test]
fn byte_limits_are_enforced_by_the_shared_decoder() {
    let input = br#"{"model":"m","query":"q","documents":[]}"#;
    let too_small = WireDecodeLimits {
        max_json_bytes: input.len().saturating_sub(1),
        max_event_bytes: 0x0010_0000,
        max_items: 10_000,
    };
    assert!(matches!(
        decode_rerank_contracts(input, RerankRecordKind::Request, too_small),
        Err(RerankContractError::Shape(_))
    ));
}

#[rstest]
#[case::too_large(br#"{"results":[{"index":0,"relevance_score":1e100}]}"#)]
#[case::beyond_json_range(br#"{"results":[{"index":0,"relevance_score":1e999}]}"#)]
fn scores_beyond_exact_decimals_are_refused(#[case] input: &[u8]) {
    assert!(matches!(
        decode(input, RerankRecordKind::Response),
        Err(RerankContractError::ScoreOutOfRange(_))
    ));
}

#[test]
fn compatibility_deviations() {
    let Err(boundary) = decode(
        br#"{"results":[{"index":0,"relevance_score":1e100}]}"#,
        RerankRecordKind::Response,
    ) else {
        panic!("the boundary score decoded");
    };
    assert!(matches!(boundary, RerankContractError::ScoreOutOfRange(_)));
    let details = boundary.details();
    assert_eq!(details.status, Some(502_u16));
    assert_eq!(details.error_type, Some("bad_gateway"));
    assert_eq!(details.param, Some("score"));
    assert_eq!(details.code, Some("decimal_out_of_range"));
    assert_eq!(details.location, None);
    assert_eq!(Score::try_from(0.125_f64).unwrap().0.to_string(), "0.125");
    assert_eq!(Score::try_from(-0.5_f64).unwrap().0.to_string(), "-0.5");
}

#[rstest]
#[case::nan(f64::NAN)]
#[case::infinity(f64::INFINITY)]
#[case::negative_infinity(f64::NEG_INFINITY)]
#[case::beyond_exact_range(1e100_f64)]
fn non_finite_scores_are_refused(#[case] value: f64) {
    let _error = Score::try_from(value).unwrap_err();
}

#[test]
fn replays_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/rerank_contracts");
    let decoded = fs::read_dir(corpus)
        .unwrap()
        .map(|entry| fs::read(entry.unwrap().path()).unwrap())
        .filter_map(|bytes| {
            let (&kind_byte, json) = bytes.split_first()?;
            let kind = RerankRecordKind::try_from(kind_byte).ok()?;
            let _outcome = decode_rerank_contracts(json, kind, limits());
            Some(())
        })
        .count();
    assert!(decoded >= 4);
}
