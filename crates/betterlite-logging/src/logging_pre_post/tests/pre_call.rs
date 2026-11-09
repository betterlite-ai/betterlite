use rstest::rstest;

use super::support::{CALL, Knob, Look, look, run_pre};

const MASKED_BASE: &str = r#""https://host/v1?key=*****1234""#;
const ORIGINAL_START: &str = r#""2026-01-01T00:00:05+00:00""#;

#[rstest]
#[case::records_the_input("", CALL, &[], Look::Detail(&["input"]), r#"[{"role": "user", "content": "call input"}]"#)]
#[case::records_the_event_type("", CALL, &[], Look::Detail(&["log_event_type"]), r#""pre_api_call""#)]
#[case::stored_header_is_masked("", CALL, &[], Look::Detail(&["additional_args", "headers", "Authorization"]), r#""Be****56""#)]
#[case::plain_header_is_kept("", CALL, &[], Look::Detail(&["additional_args", "headers", "x-trace"]), r#""t1""#)]
#[case::stored_api_base_is_masked("", CALL, &[], Look::Detail(&["additional_args", "api_base"]), MASKED_BASE)]
#[case::bearer_value_never_stored("", CALL, &[], Look::Leaks("sk-live-abcdef123456"), "clean")]
#[case::api_key_query_never_stored("", CALL, &[], Look::Leaks("abcdef1234"), "clean")]
#[case::litellm_params_api_base_is_masked("", CALL, &[], Look::Param("api_base"), MASKED_BASE)]
#[case::details_api_base_is_masked("", CALL, &[], Look::Detail(&["litellm_params", "api_base"]), MASKED_BASE)]
#[case::missing_api_base_is_empty("", r#"{"input":"x"}"#, &[], Look::Param("api_base"), r#""""#)]
#[case::call_model_replaces_the_logged_one("", r#"{"input":"x","model":"gpt-4o-mini"}"#, &[], Look::Detail(&["model"]), r#""gpt-4o-mini""#)]
#[case::empty_call_model_keeps_the_logged_one("", r#"{"input":"x","model":""}"#, &[], Look::Detail(&["model"]), r#""gpt-4o""#)]
#[case::missing_call_model_keeps_the_logged_one("", r#"{"input":"x"}"#, &[], Look::Detail(&["model"]), r#""gpt-4o""#)]
#[case::call_start_is_stamped("", CALL, &[], Look::Detail(&["api_call_start_time"]), ORIGINAL_START)]
#[case::first_call_start_is_stamped("", CALL, &[], Look::Detail(&["first_api_call_start_time"]), ORIGINAL_START)]
#[case::retry_keeps_the_first_start("", CALL, &[Knob::EarlierAttempt], Look::Detail(&["first_api_call_start_time"]), r#""earlier""#)]
#[case::retry_moves_the_latest_start("", CALL, &[Knob::EarlierAttempt], Look::Detail(&["api_call_start_time"]), ORIGINAL_START)]
#[case::plain_call_succeeds("", CALL, &[], Look::Outcome, "Ok(PreCallOutput { classifier_input: None })")]
fn logging_pre_post_behavior_tests_behavior(
    #[case] extra: &str,
    #[case] call: &str,
    #[case] knobs: &[Knob],
    #[case] what: Look<'_>,
    #[case] expected: &str,
) {
    assert_eq!(look(&run_pre(extra, call, knobs), &what), expected);
}
