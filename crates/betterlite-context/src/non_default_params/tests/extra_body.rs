#![cfg(test)]

use betterlite_models::controls::extra_body::{
    ExtraBodyError, MAX_EXTRA_BODY_BYTES, MAX_EXTRA_BODY_DEPTH,
};
use rstest::rstest;

use super::params;
use crate::non_default_params::Params;
use crate::non_default_params::error::NonDefaultParamsError;
use crate::non_default_params::extra_body::{
    ProviderSpecificRequest, add_provider_specific_params,
};

const OPENAI_PARAMS: &[&str] = &["temperature", "model"];
const NESTED: &str = r#"{"temperature": 1, "top_k": 3, "unset": null, "extra_body": {"a": 1}}"#;
const FLAT: &str = r#"{"temperature": 1, "top_k": 3, "unset": null, "dropped": 4}"#;

fn add(
    optional: &str,
    passed: &str,
    provider: &str,
    dropped: &[&str],
) -> Result<Params, NonDefaultParamsError> {
    let drops: Vec<String> = dropped.iter().map(ToString::to_string).collect();
    add_provider_specific_params(
        params(optional),
        &ProviderSpecificRequest {
            passed_params: &params(passed),
            custom_llm_provider: provider,
            openai_params: OPENAI_PARAMS,
            additional_drop_params: &drops,
        },
    )
}

#[rstest]
#[case::openai("openai", "{}", NESTED, &[], r#"{"extra_body": {"a": 1, "top_k": 3}}"#)]
#[case::azure("azure", "{}", NESTED, &[], r#"{"extra_body": {"a": 1, "top_k": 3}}"#)]
#[case::text_completion_openai("text-completion-openai", "{}", NESTED, &[], r#"{"extra_body": {"a": 1, "top_k": 3}}"#)]
#[case::openai_compatible_groq("groq", "{}", NESTED, &[], r#"{"extra_body": {"a": 1, "top_k": 3}}"#)]
#[case::openai_compatible_deepseek("deepseek", "{}", NESTED, &[], r#"{"extra_body": {"a": 1, "top_k": 3}}"#)]
#[case::anthropic("anthropic", r#"{"kept": true}"#, FLAT, &["dropped"], r#"{"kept": true, "top_k": 3}"#)]
#[case::bedrock("bedrock", r#"{"kept": true}"#, FLAT, &["dropped"], r#"{"kept": true, "top_k": 3}"#)]
#[case::ollama("ollama", r#"{"kept": true}"#, FLAT, &["dropped"], r#"{"kept": true, "top_k": 3}"#)]
#[case::empty_provider("", r#"{"kept": true}"#, FLAT, &["dropped"], r#"{"kept": true, "top_k": 3}"#)]
fn extra_body_unknown_params_go_where_the_provider_takes_them(
    #[case] provider: &str,
    #[case] optional: &str,
    #[case] passed: &str,
    #[case] dropped: &[&str],
    #[case] expected: &str,
) {
    let result = add(optional, passed, provider, dropped).unwrap();
    assert_eq!(result, params(expected));
}

#[test]
fn extra_body_other_providers_keep_a_passed_extra_body_as_a_plain_param() {
    let result = add("{}", r#"{"extra_body": {"a": 1}}"#, "anthropic", &[]).unwrap();
    assert_eq!(result, params(r#"{"extra_body": {"a": 1}}"#));
}

#[test]
fn extra_body_dropped_by_the_caller_skips_the_whole_step() {
    let result = add(
        r#"{"x": 1}"#,
        r#"{"top_k": 3, "extra_body": {"a": 1}}"#,
        "openai",
        &["extra_body"],
    )
    .unwrap();
    assert_eq!(result, params(r#"{"x": 1}"#));
}

#[test]
fn extra_body_routing_key_is_removed_wherever_it_came_from() {
    let result = add(
        r#"{"extra_body": {"model": "from-optional", "o": 1}}"#,
        r#"{"extra_body": {"model": "from-body", "b": 2}}"#,
        "openai",
        &[],
    )
    .unwrap();
    assert_eq!(result, params(r#"{"extra_body": {"o": 1, "b": 2}}"#));
}

#[test]
fn extra_body_additional_drop_params_removes_matching_keys_only() {
    let result = add(
        "{}",
        r#"{"top_k": 3, "keep": 2, "extra_body": {"top_k": 9, "also": 1}}"#,
        "openai",
        &["top_k", "also"],
    )
    .unwrap();
    assert_eq!(result, params(r#"{"extra_body": {"keep": 2}}"#));
}

#[rstest]
#[case::optional_extra_body_is_the_base(
    r#"{"extra_body": {"a": 1, "b": 1}}"#,
    r#"{"extra_body": {"b": 2}}"#,
    r#"{"extra_body": {"a": 1, "b": 2}}"#
)]
#[case::unknown_param_beats_extra_body_entry(
    "{}",
    r#"{"extra_body": {"c": 1}, "c": 2}"#,
    r#"{"extra_body": {"c": 2}}"#
)]
#[case::non_object_optional_extra_body_is_replaced(
    r#"{"extra_body": "text"}"#,
    r#"{"top_k": 1}"#,
    r#"{"extra_body": {"top_k": 1}}"#
)]
#[case::null_extra_body_is_empty("{}", r#"{"extra_body": null}"#, r#"{"extra_body": {}}"#)]
#[case::nothing_passed_still_sets_an_empty_extra_body("{}", "{}", r#"{"extra_body": {}}"#)]
#[case::other_optional_params_stay(
    r#"{"x": 1}"#,
    r#"{"top_k": 1}"#,
    r#"{"x": 1, "extra_body": {"top_k": 1}}"#
)]
fn extra_body_merge_order(#[case] optional: &str, #[case] passed: &str, #[case] expected: &str) {
    let result = add(optional, passed, "openai", &[]).unwrap();
    assert_eq!(result, params(expected));
}

#[test]
fn extra_body_that_is_not_an_object_is_rejected() {
    let error = add("{}", r#"{"extra_body": "text"}"#, "openai", &[]).unwrap_err();
    assert!(matches!(
        error,
        NonDefaultParamsError::WrongType("extra_body", _)
    ));
    assert_eq!(error.status_code(), 400);
}

// `{"big":""}` is ten bytes, so the string length sets the serialized size exactly.
#[rstest]
#[case::at_the_limit(MAX_EXTRA_BODY_BYTES - 10, true)]
#[case::one_byte_over(MAX_EXTRA_BODY_BYTES - 9, false)]
fn extra_body_size_limit_is_exact(#[case] text_len: usize, #[case] accepted: bool) {
    let big = "x".repeat(text_len);
    let passed = format!(r#"{{"extra_body": {{"big": "{big}"}}}}"#);
    let result = add("{}", &passed, "openai", &[]);
    if accepted {
        result.unwrap();
        return;
    }
    let error = result.unwrap_err();
    assert!(matches!(
        error,
        NonDefaultParamsError::ExtraBody(ExtraBodyError::TooLarge { .. })
    ));
    assert_eq!(error.status_code(), 400);
}

#[rstest]
#[case::at_the_limit(MAX_EXTRA_BODY_DEPTH, true)]
#[case::one_level_over(MAX_EXTRA_BODY_DEPTH + 1, false)]
fn extra_body_depth_limit_is_exact(#[case] levels: usize, #[case] accepted: bool) {
    let nested = format!("{}1{}", r#"{"a":"#.repeat(levels), "}".repeat(levels));
    let passed = format!(r#"{{"extra_body": {nested}}}"#);
    let result = add("{}", &passed, "openai", &[]);
    if accepted {
        result.unwrap();
        return;
    }
    assert!(matches!(
        result.unwrap_err(),
        NonDefaultParamsError::ExtraBody(ExtraBodyError::TooDeep { .. })
    ));
}

#[test]
fn extra_body_the_safety_check_is_not_run_for_other_providers() {
    let big = "x".repeat(MAX_EXTRA_BODY_BYTES);
    let passed = format!(r#"{{"big": "{big}"}}"#);
    add("{}", &passed, "anthropic", &[]).unwrap();
}
