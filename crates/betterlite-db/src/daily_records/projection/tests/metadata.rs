#![cfg(test)]

use std::str::FromStr as _;

use betterlite_models::savings_evidence::cache_extraction::extract_cache_read_tokens;
use betterlite_models::savings_evidence::compression::compression_saved_tokens;
use betterlite_models::savings_evidence::numeric::numeric_savings;
use betterlite_models::tiered_pricing::Scalar;
use rstest::rstest;
use rust_decimal::Decimal;

use crate::daily_records::metadata::{
    DailyMetadataError, RequestStatus, decode_daily_metadata, request_status,
};
use crate::redis_spend::codec::MAX_SPEND_BUFFER_BYTES;

#[rstest]
#[case::text(r#"{"internal_call_origin":"autorouter_classifier"}"#, true)]
#[case::zero_text(r#"{"internal_call_origin":"0"}"#, true)]
#[case::number(r#"{"internal_call_origin":5}"#, true)]
#[case::boolean(r#"{"internal_call_origin":true}"#, true)]
#[case::list(r#"{"internal_call_origin":[0]}"#, true)]
#[case::object(r#"{"internal_call_origin":{"a":1}}"#, true)]
#[case::empty(r#"{"internal_call_origin":""}"#, false)]
#[case::zero(r#"{"internal_call_origin":0}"#, false)]
#[case::disabled(r#"{"internal_call_origin":false}"#, false)]
#[case::null(r#"{"internal_call_origin":null}"#, false)]
#[case::empty_list(r#"{"internal_call_origin":[]}"#, false)]
#[case::empty_object(r#"{"internal_call_origin":{}}"#, false)]
#[case::absent("{}", false)]
fn reads_an_internal_call_from_a_truthy_origin(#[case] text: &str, #[case] internal: bool) {
    let metadata = decode_daily_metadata(text.as_bytes()).unwrap();
    assert_eq!(metadata.internal_call, internal);
}

#[rstest]
#[case::null(r#"{"usage_object":null}"#)]
#[case::zero(r#"{"usage_object":0}"#)]
#[case::empty_text(r#"{"usage_object":""}"#)]
#[case::disabled(r#"{"usage_object":false}"#)]
#[case::empty_list(r#"{"usage_object":[]}"#)]
#[case::empty_object(r#"{"usage_object":{}}"#)]
#[case::absent("{}")]
fn a_falsey_usage_object_reads_as_absent(#[case] text: &str) {
    let metadata = decode_daily_metadata(text.as_bytes()).unwrap();
    assert!(metadata.usage_object.is_none());
    assert!(metadata.raw_usage.is_none());
}

#[test]
fn reads_the_usage_object_both_ways() {
    let metadata =
        decode_daily_metadata(br#"{"usage_object":{"cache_read_input_tokens":4}}"#).unwrap();
    assert_eq!(
        extract_cache_read_tokens(metadata.usage_object.as_ref()).unwrap(),
        4
    );
    assert!(metadata.raw_usage.is_some());
}

#[rstest]
#[case::usage_list(r#"{"usage_object":[1]}"#, "usage_object")]
#[case::usage_text(r#"{"usage_object":"x"}"#, "usage_object")]
#[case::usage_number(r#"{"usage_object":5}"#, "usage_object")]
#[case::usage_boolean(r#"{"usage_object":true}"#, "usage_object")]
#[case::cost_text(r#"{"cost_breakdown":"batch"}"#, "cost_breakdown")]
#[case::cost_list(r#"{"cost_breakdown":[1]}"#, "cost_breakdown")]
fn a_truthy_non_object_metadata_field_fails_the_row(#[case] text: &str, #[case] field: &str) {
    let result = decode_daily_metadata(text.as_bytes());
    match result {
        Err(DailyMetadataError::NotAnObject { field: reported }) => assert_eq!(reported, field),
        other => panic!("expected a non-object field error, got {other:?}"),
    }
}

#[test]
fn reads_a_cost_breakdown_only_when_it_holds_something() {
    let empty = decode_daily_metadata(br#"{"cost_breakdown":{}}"#).unwrap();
    assert!(empty.cost_breakdown.is_none());
    let filled = decode_daily_metadata(br#"{"cost_breakdown":{"service_tier":"batch"}}"#).unwrap();
    let breakdown = filled.cost_breakdown.expect("the breakdown is read");
    assert!(matches!(breakdown.service_tier, Some(Scalar::Text(_))));
}

#[rstest]
#[case::empty("")]
#[case::list("[]")]
#[case::number("5")]
#[case::null("null")]
#[case::text(r#""x""#)]
fn rejects_metadata_that_is_not_an_object(#[case] text: &str) {
    let result = decode_daily_metadata(text.as_bytes());
    assert!(matches!(result, Err(DailyMetadataError::NotObject)));
}

#[test]
fn accepts_leading_whitespace_before_the_object() {
    let metadata = decode_daily_metadata(b"  \n{}").unwrap();
    assert!(!metadata.internal_call);
}

#[test]
fn rejects_invalid_json() {
    let result = decode_daily_metadata(b"{");
    assert!(matches!(result, Err(DailyMetadataError::Json(_))));
}

#[test]
fn rejects_metadata_over_the_spend_record_bound() {
    let oversized = vec![b' '; MAX_SPEND_BUFFER_BYTES.saturating_add(1)];
    assert!(matches!(
        decode_daily_metadata(&oversized),
        Err(DailyMetadataError::TooLarge)
    ));
}

#[test]
fn keeps_metadata_of_exactly_the_byte_ceiling() {
    let at_limit = vec![b' '; MAX_SPEND_BUFFER_BYTES];
    assert!(matches!(
        decode_daily_metadata(&at_limit),
        Err(DailyMetadataError::NotObject)
    ));
}

#[test]
fn reads_the_savings_view_of_the_metadata() {
    let metadata = decode_daily_metadata(
        br#"{"compression_savings":{"tokens_saved":12},"autorouter_savings":0.5}"#,
    )
    .unwrap();
    assert_eq!(compression_saved_tokens(&metadata.savings).unwrap(), 12);
    assert_eq!(
        numeric_savings(metadata.savings.autorouter_savings.as_ref()),
        Some(Decimal::from_str("0.5").unwrap())
    );
}

#[rstest]
#[case::text(r#"{"routing_decision":"x"}"#)]
#[case::number(r#"{"routing_decision":5}"#)]
#[case::list(r#"{"routing_decision":[1]}"#)]
#[case::same_arity_list(r#"{"routing_decision":[1,"gpt-4o",null]}"#)]
#[case::null(r#"{"routing_decision":null}"#)]
#[case::absent("{}")]
fn ignores_a_non_mapping_routing_decision(#[case] text: &str) {
    let metadata = decode_daily_metadata(text.as_bytes()).unwrap();
    assert!(metadata.routing_decision.is_none());
}

#[test]
fn reads_a_mapping_routing_decision() {
    let metadata =
        decode_daily_metadata(br#"{"routing_decision":{"classifier_cost":0.25}}"#).unwrap();
    let decision = metadata.routing_decision.expect("the decision is read");
    assert!(matches!(decision.classifier_cost, Some(Scalar::Number(_))));
}

#[rstest]
#[case::failure(r#"{"status":"failure"}"#, RequestStatus::Failure)]
#[case::success(r#"{"status":"success"}"#, RequestStatus::Success)]
#[case::empty("{}", RequestStatus::Success)]
#[case::capitalized(r#"{"status":"Failure"}"#, RequestStatus::Success)]
#[case::boolean(r#"{"status":false}"#, RequestStatus::Success)]
#[case::number(r#"{"status":5}"#, RequestStatus::Success)]
#[case::not_json("not json", RequestStatus::Success)]
#[case::list("[1]", RequestStatus::Success)]
#[case::text(r#""failure""#, RequestStatus::Success)]
fn the_status_comes_from_a_failure_text_in_the_metadata(
    #[case] text: &str,
    #[case] status: RequestStatus,
) {
    assert_eq!(request_status(Some(text.as_bytes())), status);
}

#[test]
fn an_absent_metadata_reads_as_a_success() {
    assert_eq!(request_status(None), RequestStatus::Success);
}
