use rstest::rstest;

use super::support::{TPOT, TTFT, USAGE_1_4, USAGE_2_4, record, record_result, sums};

#[rstest]
#[case::streaming(
    r#"{"optional_params": {"stream": true}, "api_call_start_time": 10.5, "completion_start_time": 11.25}"#,
    &["0.75"]
)]
#[case::not_streaming(
    r#"{"optional_params": {"stream": false}, "api_call_start_time": 10.5, "completion_start_time": 11.25}"#,
    &[]
)]
#[case::stream_missing(r#"{"api_call_start_time": 10.5, "completion_start_time": 11.25}"#, &[])]
#[case::no_first_token(
    r#"{"optional_params": {"stream": true}, "api_call_start_time": 10.5}"#,
    &[]
)]
#[case::no_api_start(
    r#"{"optional_params": {"stream": true}, "completion_start_time": 11.25}"#,
    &[]
)]
#[case::unparsed_text(
    r#"{"optional_params": {"stream": true}, "api_call_start_time": "not a time", "completion_start_time": 11.25}"#,
    &[]
)]
#[case::first_token_before_the_start(
    r#"{"optional_params": {"stream": true}, "api_call_start_time": 11, "completion_start_time": 10.5}"#,
    &["-0.5"]
)]
fn time_to_first_token_measures_streaming_calls(#[case] extra: &str, #[case] expected: &[&str]) {
    let recorded = record(extra, None);
    assert_eq!(sums(&recorded, TTFT), expected);
}

#[rstest]
#[case::streaming_uses_the_first_token(
    r#"{"api_call_start_time": 10.5, "completion_start_time": 11}"#,
    "0.5"
)]
#[case::non_streaming_uses_the_api_start(r#"{"api_call_start_time": 11}"#, "0.5")]
#[case::neither_uses_the_whole_duration("{}", "0.75")]
#[case::unparsed_first_token_uses_the_duration(
    r#"{"completion_start_time": "later", "api_call_start_time": 11}"#,
    "0.75"
)]
#[case::unparsed_api_start_uses_the_duration(r#"{"api_call_start_time": "later"}"#, "0.75")]
fn time_per_output_token_divides_the_generation_time_by_completion_tokens(
    #[case] extra: &str,
    #[case] expected: &str,
) {
    let recorded = record(extra, Some(USAGE_1_4));
    assert_eq!(sums(&recorded, TPOT), [expected]);
}

#[rstest]
#[case::zero_tokens(r#"{"usage": {"completion_tokens": 0}}"#)]
#[case::negative_tokens(r#"{"usage": {"completion_tokens": -2}}"#)]
#[case::no_tokens(r#"{"usage": {"prompt_tokens": 3}}"#)]
#[case::no_usage(r#"{"id": "x"}"#)]
fn time_per_output_token_needs_positive_completion_tokens(#[case] response: &str) {
    let recorded = record(r#"{"api_call_start_time": 11}"#, Some(response));
    assert!(sums(&recorded, TPOT).is_empty());
}

#[rstest]
#[case::zero_generation(r#"{"api_call_start_time": 11, "completion_start_time": 13}"#)]
#[case::negative_generation(r#"{"api_call_start_time": 11, "completion_start_time": 14}"#)]
fn time_per_output_token_skips_a_generation_time_that_is_not_positive(#[case] extra: &str) {
    let recorded = record(extra, Some(USAGE_2_4));
    assert!(sums(&recorded, TPOT).is_empty());
}

#[test]
fn time_per_output_token_is_skipped_without_a_response() {
    let recorded = record(r#"{"api_call_start_time": 11}"#, None);
    assert!(sums(&recorded, TPOT).is_empty());
}

#[test]
fn a_completion_start_time_of_the_wrong_type_is_an_error_naming_the_field() {
    let error = record_result(
        r#"{"completion_start_time": {"seconds": 1}}"#,
        Some(USAGE_1_4),
    )
    .expect_err("wrong type");
    assert!(error.to_string().contains("completion_start_time"));
}
