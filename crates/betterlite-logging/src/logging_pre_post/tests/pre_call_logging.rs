use rstest::rstest;

use super::support::{CALL, Knob, Look, look, run_pre};

const CURL: &str = "\n\nPOST Request Sent from LiteLLM:\ncurl -X POST \\\nhttps://host/v1?REDACTED \\\n-H 'Authorization: Be****56' -H 'x-trace: t1' \\\n-d '{\"model\": \"gpt-4o\"}'\n";
const MANUAL: &str = r#"{"input":"x","additional_args":{"request_str":"manual request"}}"#;
const JSON_FIELDS: &str =
    r#"{"api_base": "https://host/v1?REDACTED", "Authorization": "Be****56", "x-trace": "t1"}"#;

#[rstest]
#[case::quiet_by_default(CALL, &[], "none")]
#[case::verbose_prints_the_curl_at_debug(CALL, &[Knob::Verbose], &format!("Debug:\x1b[92m{CURL}\x1b[0m\n:{{}}"))]
#[case::request_debug_prints_at_warning(CALL, &[Knob::RequestDebug], &format!("Warning:\x1b[92m{CURL}\x1b[0m\n:{{}}"))]
#[case::request_debug_wins_over_verbose(CALL, &[Knob::Verbose, Knob::RequestDebug], &format!("Warning:\x1b[92m{CURL}\x1b[0m\n:{{}}"))]
#[case::json_logs_alone_stay_quiet(CALL, &[Knob::JsonLogs], "none")]
#[case::json_logs_carry_masked_fields(CALL, &[Knob::Verbose, Knob::JsonLogs], &format!("Debug:POST Request Sent from LiteLLM:{JSON_FIELDS}"))]
#[case::json_logs_warn_for_request_debug(CALL, &[Knob::RequestDebug, Knob::JsonLogs], &format!("Warning:POST Request Sent from LiteLLM:{JSON_FIELDS}"))]
#[case::request_str_replaces_the_curl(MANUAL, &[Knob::Verbose], "Debug:\x1b[92m\nRequest Sent from LiteLLM:\nmanual request\x1b[0m\n:{}")]
fn logging_pre_post_behavior_tests_behavior(
    #[case] call: &str,
    #[case] knobs: &[Knob],
    #[case] expected: &str,
) {
    assert_eq!(look(&run_pre("", call, knobs), &Look::Record(0)), expected);
}
