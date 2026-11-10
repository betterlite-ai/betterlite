use rstest::rstest;

use super::support::{CALL, Knob, Look, look, run_pre};

const STATE_FLAG: &str = r#","request_logging":{"log_raw_request_response":true}"#;
const METADATA: &str = r#","litellm_params":{"metadata":{"user":"u1"}}"#;
const EMPTY_METADATA: &str = r#","litellm_params":{"metadata":{}}"#;
const TYPED: &str = r#"{"raw_request_api_base": "https://host/v1?key=*****1234", "raw_request_body": {"model": "gpt-4o"}, "raw_request_headers": {"Authorization": "Be****56", "x-trace": "t1"}, "error": null}"#;
const HIDDEN: &str = r#"{"raw_request_api_base": "https://host/v1?key=*****1234", "raw_request_body": "redacted-by-litellm", "raw_request_headers": {"Authorization": "Be****56", "x-trace": "t1"}, "error": null}"#;
const STORED: &str = r#"{"user": "u1", "raw_request": "\n\nPOST Request Sent from LiteLLM:\ncurl -X POST \\\nhttps://host/v1?REDACTED \\\n-H 'Authorization: Be****56' -H 'x-trace: t1' \\\n-d '{\"model\": \"gpt-4o\"}'\n"}"#;

#[rstest]
#[case::off_by_default("", CALL, &[], Look::Detail(&["raw_request_typed_dict"]), "absent")]
#[case::state_flag_records_it(STATE_FLAG, CALL, &[], Look::Detail(&["raw_request_typed_dict"]), TYPED)]
#[case::global_flag_records_it("", CALL, &[Knob::GlobalRawLogging], Look::Detail(&["raw_request_typed_dict"]), TYPED)]
#[case::missing_args_record_empty_parts("", r#"{"input":"x"}"#, &[Knob::GlobalRawLogging], Look::Detail(&["raw_request_typed_dict"]), r#"{"raw_request_api_base": "", "raw_request_body": {}, "raw_request_headers": {}, "error": null}"#)]
#[case::hidden_content_hides_the_body("", CALL, &[Knob::GlobalRawLogging, Knob::RedactContent], Look::Detail(&["raw_request_typed_dict"]), HIDDEN)]
#[case::metadata_gets_the_curl(METADATA, CALL, &[Knob::GlobalRawLogging], Look::Param("metadata"), STORED)]
#[case::metadata_gets_the_redacted_text(METADATA, CALL, &[Knob::GlobalRawLogging, Knob::RedactContent], Look::Param("metadata"), r#"{"user": "u1", "raw_request": "redacted-by-litellm"}"#)]
#[case::metadata_untouched_when_off(METADATA, CALL, &[], Look::Param("metadata"), r#"{"user": "u1"}"#)]
#[case::empty_metadata_gets_nothing(EMPTY_METADATA, CALL, &[Knob::GlobalRawLogging], Look::Param("metadata"), "{}")]
#[case::missing_metadata_gets_nothing("", CALL, &[Knob::GlobalRawLogging], Look::Param("metadata"), "absent")]
#[case::details_metadata_follows(METADATA, CALL, &[Knob::GlobalRawLogging], Look::Detail(&["litellm_params", "metadata", "user"]), r#""u1""#)]
fn logging_pre_post_behavior_tests_behavior(
    #[case] extra: &str,
    #[case] call: &str,
    #[case] knobs: &[Knob],
    #[case] what: Look<'_>,
    #[case] expected: &str,
) {
    assert_eq!(look(&run_pre(extra, call, knobs), &what), expected);
}
