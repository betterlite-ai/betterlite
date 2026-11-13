use rstest::rstest;

use super::support::{CALL, Knob, PreRun, run_pre};
use crate::python_json;

const CLASSIFIER: &str =
    r#","litellm_params":{"metadata":{"internal_call_origin":"autorouter_classifier"}}"#;

const SDK_CALL: &str = r#"{"input":"x","additional_args":{"openai_sdk":true,"complete_input_dict":{"model":"gpt-4o","extra_body":{"a":1}}}}"#;
const NOT_SDK_CALL: &str = r#"{"input":"x","additional_args":{"openai_sdk":false,"complete_input_dict":{"model":"gpt-4o","extra_body":{"a":1}}}}"#;
const UNSET_SDK_CALL: &str = r#"{"input":"x","additional_args":{"complete_input_dict":{"model":"gpt-4o","extra_body":{"a":1}}}}"#;
const KEPT: &str = r#"{"model": "gpt-4o", "extra_body": {"a": 1}}"#;

fn classifier_input(run: &PreRun) -> String {
    match &run.result {
        Ok(output) => output
            .classifier_input
            .as_ref()
            .map_or_else(|| "none".to_owned(), python_json::dumps_object),
        Err(error) => error.to_string(),
    }
}

#[rstest]
#[case::captured_for_a_classifier_call(CLASSIFIER, CALL, &[], r#"{"model": "gpt-4o"}"#)]
#[case::withheld_when_content_is_hidden(CLASSIFIER, CALL, &[Knob::RedactContent], "none")]
#[case::none_for_a_plain_call("", CALL, &[], "none")]
#[case::sdk_flag_merges_extra_body(CLASSIFIER, SDK_CALL, &[], r#"{"model": "gpt-4o", "a": 1}"#)]
#[case::sdk_flag_false_keeps_extra_body(CLASSIFIER, NOT_SDK_CALL, &[], KEPT)]
#[case::sdk_flag_unset_keeps_extra_body(CLASSIFIER, UNSET_SDK_CALL, &[], KEPT)]
fn logging_pre_post_behavior_tests_behavior(
    #[case] extra: &str,
    #[case] call: &str,
    #[case] knobs: &[Knob],
    #[case] expected: &str,
) {
    assert_eq!(classifier_input(&run_pre(extra, call, knobs)), expected);
}
