#![cfg(test)]

use std::path::Path;

use rstest::rstest;

use betterlite_oracle_runner::case::load_helper_cases;
use betterlite_oracle_runner::helpers::{HelperRun, ORACLE_DIR, run_case};

#[rstest]
#[case::config_empty_litellm_params("config-empty-litellm-params")]
#[case::config_missing_litellm_params("config-missing-litellm-params")]
#[case::config_present_litellm_params("config-present-litellm-params")]
#[case::envelope_jsonrpc_2_0("envelope-jsonrpc-2-0")]
#[case::envelope_method_params("envelope-method-params")]
#[case::filter_no_headers("filter-no-headers")]
#[case::filter_reserved_case_insensitive("filter-reserved-case-insensitive")]
#[case::filter_reserved_exact("filter-reserved-exact")]
#[case::filter_reserved_prefix("filter-reserved-prefix")]
#[case::filter_safe_headers("filter-safe-headers")]
#[case::model_bedrock_prefix_strip("model-bedrock-prefix-strip")]
#[case::model_no_bedrock_prefix("model-no-bedrock-prefix")]
#[case::non_streaming_error_response("non-streaming-error-response")]
#[case::non_streaming_json_response("non-streaming-json-response")]
#[case::session_contextid_missing("session-contextid-missing")]
#[case::session_contextid_not_string("session-contextid-not-string")]
#[case::session_contextid_present("session-contextid-present")]
#[case::sse_empty_data_skip("sse-empty-data-skip")]
#[case::sse_invalid_json_skip("sse-invalid-json-skip")]
#[case::sse_valid_json_lines("sse-valid-json-lines")]
#[case::streaming_json_content_type("streaming-json-content-type")]
#[case::streaming_sse_content_type("streaming-sse-content-type")]
#[case::validate_session_256_chars("validate-session-256-chars")]
#[case::validate_session_257_chars("validate-session-257-chars")]
#[case::validate_session_32_chars("validate-session-32-chars")]
#[case::validate_session_33_chars("validate-session-33-chars")]
fn agentcore_transform_cases(#[case] name: &str) {
    let path = Path::new(ORACLE_DIR).join(format!("cases/helpers/agentcore-transform/{name}.yaml"));
    let cases = load_helper_cases(&[path]).unwrap();
    let case = &cases[0];
    assert!(matches!(
        run_case(&[crate::GROUP], case, false).unwrap(),
        HelperRun::Match
    ));
}
