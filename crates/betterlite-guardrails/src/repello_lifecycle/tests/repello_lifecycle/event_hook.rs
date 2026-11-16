use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_guardrails::repello_lifecycle::config::{
    RepelloLifecycleError, event_hook_from_mode,
};
use rstest::rstest;

fn decoded(mode: &str) -> LitellmParamsMode {
    serde_json::from_str(mode).unwrap()
}

#[rstest]
#[case::single(r#""pre_call""#)]
#[case::post_call(r#""post_call""#)]
#[case::list(r#"["pre_call", "post_call"]"#)]
#[case::tagged(r#"{"tags": {"chat": "pre_call"}, "default": "post_call"}"#)]
#[case::tagged_list(r#"{"tags": {"chat": ["pre_call", "post_call"]}}"#)]
fn a_valid_mode_comes_back_unchanged(#[case] mode: &str) {
    let mode = decoded(mode);
    let validated = event_hook_from_mode(&mode).unwrap();
    assert_eq!(validated, mode);
}

// LiteLLM returns a tagged Mode untouched, so a name it would reject in a string or a list stays
#[rstest]
#[case::bad_tagged(r#"{"tags": {"chat": "nope"}}"#)]
#[case::bad_tagged_list(r#"{"tags": {"chat": ["pre_call", "nope"]}}"#)]
#[case::bad_tagged_default(r#"{"tags": {"chat": "pre_call"}, "default": "nope"}"#)]
fn a_tagged_mode_with_an_unknown_hook_comes_back_unchanged(#[case] mode: &str) {
    let mode = decoded(mode);
    assert_eq!(event_hook_from_mode(&mode).unwrap(), mode);
}

#[rstest]
#[case::unknown(r#""nope""#)]
#[case::wrong_case(r#""PreCall""#)]
#[case::empty(r#""""#)]
#[case::one_bad_in_a_list(r#"["pre_call", "nope"]"#)]
fn an_unknown_event_hook_is_rejected(#[case] mode: &str) {
    let error = event_hook_from_mode(&decoded(mode)).unwrap_err();
    assert!(matches!(
        &error,
        RepelloLifecycleError::InvalidEventHook { .. }
    ));
    assert!(error.to_string().contains("is not a guardrail event hook"));
}

#[rstest]
#[case::pre_call("pre_call")]
#[case::post_call("post_call")]
#[case::during_call("during_call")]
#[case::logging_only("logging_only")]
#[case::pre_mcp_call("pre_mcp_call")]
#[case::during_mcp_call("during_mcp_call")]
#[case::post_mcp_call("post_mcp_call")]
#[case::realtime("realtime_input_transcription")]
fn every_guardrail_event_hook_name_is_accepted(#[case] name: &str) {
    let mode = LitellmParamsMode::One(name.to_owned());
    assert_eq!(event_hook_from_mode(&mode).unwrap(), mode);
}
