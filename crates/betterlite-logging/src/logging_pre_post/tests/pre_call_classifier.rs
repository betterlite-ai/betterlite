use rstest::rstest;

use super::support::{CALL, Knob, PreRun, run_pre};
use crate::python_json;

const CLASSIFIER: &str =
    r#","litellm_params":{"metadata":{"internal_call_origin":"autorouter_classifier"}}"#;

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
#[case::captured_for_a_classifier_call(CLASSIFIER, &[], r#"{"model": "gpt-4o"}"#)]
#[case::withheld_when_content_is_hidden(CLASSIFIER, &[Knob::RedactContent], "none")]
#[case::none_for_a_plain_call("", &[], "none")]
fn logging_pre_post_behavior_tests_behavior(
    #[case] extra: &str,
    #[case] knobs: &[Knob],
    #[case] expected: &str,
) {
    assert_eq!(classifier_input(&run_pre(extra, CALL, knobs)), expected);
}
