use rstest::rstest;

use super::support::{COST, DURATION, RESPONSE, TOKENS, TPOT, TTFT, record, sums};

#[rstest]
#[case::get_responses("get_responses", "", "", false)]
#[case::vector_store_list("avector_store_list", "", "", false)]
#[case::background_response("get_responses", r#", "background": true"#, "", true)]
#[case::background_false("get_responses", r#", "background": false"#, "", false)]
#[case::cost_poll_metadata(
    "get_responses",
    "",
    r#", "metadata": {"internal_call_origin": "background_response_cost_poll"}"#,
    true
)]
#[case::cost_poll_litellm_metadata(
    "get_responses",
    "",
    r#", "litellm_metadata": {"internal_call_origin": "background_response_cost_poll"}"#,
    true
)]
#[case::other_origin(
    "get_responses",
    "",
    r#", "metadata": {"internal_call_origin": "shadow_eval_judge"}"#,
    false
)]
#[case::metadata_wins_over_litellm_metadata(
    "get_responses",
    "",
    r#", "metadata": {"internal_call_origin": "shadow_eval_judge"}, "litellm_metadata": {"internal_call_origin": "background_response_cost_poll"}"#,
    false
)]
#[case::inference_call("acompletion", r#", "background": false"#, "", true)]
fn unbilled_calls_skip_token_usage_and_time_per_output_token(
    #[case] call_type: &str,
    #[case] response_fields: &str,
    #[case] params_fields: &str,
    #[case] billed: bool,
) {
    let response =
        format!(r#"{{"usage": {{"prompt_tokens": 2, "completion_tokens": 4}}{response_fields}}}"#);
    let extra = format!(
        r#"{{"call_type": "{call_type}", "api_call_start_time": 10.5,
        "litellm_params": {{"custom_llm_provider": "openai"{params_fields}}}}}"#
    );
    let recorded = record(&extra, Some(response.as_str()));
    assert_eq!(!sums(&recorded, TOKENS).is_empty(), billed);
    assert_eq!(!sums(&recorded, TPOT).is_empty(), billed);
    assert_eq!(sums(&recorded, DURATION), ["3"]);
    assert_eq!(sums(&recorded, RESPONSE), ["2.5"]);
}

#[test]
fn an_unbilled_call_still_records_the_cost_and_the_time_to_first_token() {
    let extra = r#"{
        "call_type": "get_responses",
        "response_cost": 0.5,
        "optional_params": {"stream": true},
        "api_call_start_time": 10.5,
        "completion_start_time": 11.0
    }"#;
    let response = r#"{"usage": {"prompt_tokens": 2, "completion_tokens": 4}, "background": true}"#;
    let recorded = record(extra, Some(response));
    assert!(sums(&recorded, TOKENS).is_empty());
    assert!(sums(&recorded, TPOT).is_empty());
    assert_eq!(sums(&recorded, COST), ["0.5"]);
    assert_eq!(sums(&recorded, TTFT), ["0.5"]);
}
