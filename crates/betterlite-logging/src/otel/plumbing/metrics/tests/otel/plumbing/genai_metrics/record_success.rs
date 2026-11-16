use std::collections::BTreeMap;

use rstest::rstest;

use super::support::{
    COST, DURATION, RESPONSE, Recorded, TOKENS, TPOT, TTFT, USAGE_2_4, names, record,
    record_result, sums,
};

#[test]
fn a_streaming_call_records_all_six_histograms_once() {
    let extra = r#"{
        "optional_params": {"stream": true},
        "api_call_start_time": 10.5,
        "completion_start_time": 11.0,
        "response_cost": 0.5
    }"#;
    let recorded = record(extra, Some(USAGE_2_4));
    assert_eq!(sums(&recorded, DURATION), ["3"]);
    assert_eq!(sums(&recorded, TOKENS), ["2", "4"]);
    assert_eq!(sums(&recorded, COST), ["0.5"]);
    assert_eq!(sums(&recorded, TTFT), ["0.5"]);
    assert_eq!(sums(&recorded, TPOT), ["0.5"]);
    assert_eq!(sums(&recorded, RESPONSE), ["2.5"]);
    assert!(recorded.iter().all(|entry| entry.count == 1));
}

#[test]
fn an_empty_call_records_the_operation_duration_alone() {
    let recorded = record("{}", None);
    assert_eq!(names(&recorded), [DURATION]);
    assert_eq!(sums(&recorded, DURATION), ["3"]);
}

#[test]
fn a_call_without_a_response_records_the_duration_and_the_cost() {
    let recorded = record(r#"{"response_cost": 0.25}"#, None);
    assert_eq!(names(&recorded), [DURATION, COST]);
    assert_eq!(sums(&recorded, COST), ["0.25"]);
}

#[rstest]
#[case::absent_usage(r#"{"id": "x"}"#)]
#[case::empty_usage(r#"{"usage": {}}"#)]
fn a_response_without_usage_records_no_token_usage(#[case] response: &str) {
    let recorded = record("{}", Some(response));
    assert!(sums(&recorded, TOKENS).is_empty());
    assert_eq!(sums(&recorded, DURATION), ["3"]);
}

#[test]
fn missing_token_counts_record_zero() {
    let recorded = record("{}", Some(r#"{"usage": {"total_tokens": 9}}"#));
    assert_eq!(sums(&recorded, TOKENS), ["0", "0"]);
}

#[test]
fn token_usage_is_split_by_the_token_type_attribute() {
    let recorded = record("{}", Some(USAGE_2_4));
    let tokens: Vec<&Recorded> = recorded
        .iter()
        .filter(|entry| entry.name == TOKENS)
        .collect();
    let by_type: BTreeMap<&str, &str> = tokens
        .iter()
        .map(|entry| {
            (
                entry.attributes["gen_ai.token.type"].as_str(),
                entry.sum.as_str(),
            )
        })
        .collect();
    assert_eq!(by_type, BTreeMap::from([("input", "2"), ("output", "4")]));
    assert!(
        tokens
            .iter()
            .all(|entry| entry.attributes["gen_ai.system"] == "openai")
    );
}

#[rstest]
#[case::absent("{}", false)]
#[case::zero(r#"{"response_cost": 0}"#, false)]
#[case::negative(r#"{"response_cost": -0.5}"#, false)]
#[case::positive(r#"{"response_cost": 0.25}"#, true)]
fn the_cost_is_recorded_only_when_it_is_positive(#[case] extra: &str, #[case] recorded_cost: bool) {
    let recorded = record(extra, None);
    assert_eq!(!sums(&recorded, COST).is_empty(), recorded_cost);
}

#[test]
fn a_cost_that_is_not_a_number_is_an_error_naming_the_field() {
    let error = record_result(r#"{"response_cost": "much"}"#, None).expect_err("wrong type");
    assert!(error.to_string().contains("response_cost"));
}

#[test]
fn a_token_count_that_is_not_a_number_is_an_error_naming_the_field() {
    let error = record_result("{}", Some(r#"{"usage": {"completion_tokens": "many"}}"#))
        .expect_err("wrong type");
    assert!(error.to_string().contains("completion_tokens"));
}
