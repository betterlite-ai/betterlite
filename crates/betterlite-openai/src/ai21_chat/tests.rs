#![cfg(test)]

use rstest::rstest;

use super::{Ai21ChatConfig, Ai21ChatError, SUPPORTED_OPENAI_PARAMS, map_openai_params};
use crate::openai_base_config::Object;

fn object(text: &str) -> Object {
    serde_json::from_str(text).unwrap()
}

fn map(params: &str, options: &str) -> Object {
    let mut optional = object(options);
    map_openai_params(&object(params), &mut optional);
    optional
}

#[test]
fn the_supported_list_is_the_fixed_litellm_list() {
    assert_eq!(
        SUPPORTED_OPENAI_PARAMS,
        [
            "tools",
            "response_format",
            "max_tokens",
            "max_completion_tokens",
            "temperature",
            "stop",
            "n",
            "stream",
            "seed",
            "tool_choice",
        ]
    );
}

#[test]
fn a_config_starts_empty_because_every_class_value_is_none() {
    assert_eq!(Ai21ChatConfig::default().config(), Object::new());
}

#[test]
fn the_constructor_values_become_the_config_and_absent_ones_are_left_out() {
    let settings = object(
        r#"{"tools": [{"type": "function"}], "response_format": {"type": "json_object"}, "max_tokens": 7, "temperature": 0, "top_p": 1, "stop": "", "n": 2, "stream": false, "seed": 4, "tool_choice": "auto", "user": "caller"}"#,
    );
    let config = Ai21ChatConfig::try_from(settings).unwrap();
    assert_eq!(
        config.config(),
        object(
            r#"{"tools": [{"type": "function"}], "response_format": {"type": "json_object"}, "max_tokens": 7, "temperature": 0, "top_p": 1, "stop": "", "n": 2, "stream": false, "seed": 4, "tool_choice": "auto", "user": "caller"}"#,
        )
    );
}

#[rstest]
#[case::explicit_null_is_absent(r#"{"temperature": null}"#, "{}")]
#[case::empty_object_is_absent("{}", "{}")]
#[case::documents_only(
    r#"{"documents": [{"text": "a"}]}"#,
    r#"{"documents": [{"text": "a"}]}"#
)]
fn a_null_or_absent_constructor_value_stays_out_of_the_config(
    #[case] settings: &str,
    #[case] expected: &str,
) {
    let config = Ai21ChatConfig::try_from(object(settings)).unwrap();
    assert_eq!(config.config(), object(expected));
}

#[test]
fn a_constructor_key_litellm_does_not_accept_is_an_error() {
    let error = Ai21ChatConfig::try_from(object(r#"{"unknown_setting": 1}"#)).unwrap_err();
    assert!(matches!(error, Ai21ChatError::Settings { .. }));
}

#[test]
fn every_supported_parameter_is_copied_and_the_rest_are_dropped() {
    let params = r#"{"tools": [], "response_format": {"type": "json_object"}, "max_tokens": 7, "temperature": 0.2, "stop": ["END"], "n": 2, "stream": true, "seed": 4, "tool_choice": "auto", "frequency_penalty": 0.1, "top_p": 1, "user": "caller", "modalities": ["text"], "unknown": 1}"#;
    let expected = r#"{"tools": [], "response_format": {"type": "json_object"}, "max_tokens": 7, "temperature": 0.2, "stop": ["END"], "n": 2, "stream": true, "seed": 4, "tool_choice": "auto"}"#;
    assert_eq!(map(params, "{}"), object(expected));
}

#[rstest]
#[case::only_completion_tokens(r#"{"max_completion_tokens": 5}"#, "{}", r#"{"max_tokens": 5}"#)]
#[case::only_max_tokens(r#"{"max_tokens": 7}"#, "{}", r#"{"max_tokens": 7}"#)]
#[case::completion_tokens_wins(
    r#"{"max_tokens": 7, "max_completion_tokens": 9}"#,
    "{}",
    r#"{"max_tokens": 9}"#
)]
#[case::null_is_renamed(r#"{"max_completion_tokens": null}"#, "{}", r#"{"max_tokens": null}"#)]
#[case::existing_limit_without_a_request(
    "{}",
    r#"{"max_tokens": 1, "max_completion_tokens": 2}"#,
    r#"{"max_tokens": 1, "max_completion_tokens": 2}"#
)]
#[case::existing_max_tokens_is_overwritten(
    r#"{"max_completion_tokens": 2}"#,
    r#"{"max_tokens": 1}"#,
    r#"{"max_tokens": 2}"#
)]
fn the_token_limit_is_renamed_after_the_filter(
    #[case] params: &str,
    #[case] options: &str,
    #[case] expected: &str,
) {
    assert_eq!(map(params, options), object(expected));
}
