#![cfg(test)]

use super::{Input, Op};
use crate::helpers::Helper as _;
use rstest::rstest;
use serde_json::json;

#[rstest]
#[case::counts("create_and_search_records", "counts", json!(42_i32))]
#[case::status("create_and_search_records", "response_statuses", json!(["unknown"]))]
#[case::failure("create_and_search_records", "failure_modes", json!(["unknown"]))]
#[case::guardrail("knowledge_base_records", "response_guardrail", json!("unknown"))]
#[case::null_response("knowledge_base_records", "response_empty_nulls", json!(42_i32))]
#[case::integration("vector_records_round_trip", "integrations", json!(["unknown"]))]
#[case::index("vector_records_round_trip", "index_types", json!(["unknown"]))]
#[case::unmapped("knowledge_base_records", "unmapped_entry", json!({}))]
fn every_group_entry_is_decoded(
    #[case] operation: &str,
    #[case] key: &str,
    #[case] value: serde_json::Value,
) {
    let input: Input = serde_json::from_value(json!({
        "operation": operation,
        "record": {key: value},
    }))
    .unwrap();
    assert!(
        Op::run(&input).is_err(),
        "{operation}.{key} bypassed decoding"
    );
}

#[test]
fn response_nulls_survive_decoding() {
    let record = json!({"response_empty_nulls": {
        "guardrailAction": null, "nextToken": null, "retrievalResults": null,
    }});
    let input: Input = serde_json::from_value(json!({
        "operation": "knowledge_base_records", "record": record,
    }))
    .unwrap();
    assert_eq!(Op::run(&input).unwrap(), record);
}
