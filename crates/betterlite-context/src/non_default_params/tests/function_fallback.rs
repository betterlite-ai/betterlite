#![cfg(test)]

use rstest::rstest;

use super::params;
use crate::non_default_params::function_fallback::pre_process_optional_params;

#[rstest]
#[case::tools_move_and_tool_choice_goes(
    r#"{"tools": [{"type": "function"}], "tool_choice": "auto", "n": 1}"#,
    r#"{"format": "json", "functions_unsupported_model": [{"type": "function"}]}"#,
    r#"{"n": 1}"#,
    true
)]
#[case::tools_win_over_functions(
    r#"{"tools": [1], "functions": [2]}"#,
    r#"{"format": "json", "functions_unsupported_model": [1]}"#,
    r#"{"functions": [2]}"#,
    true
)]
#[case::functions_move(
    r#"{"functions": [2], "tool_choice": "auto"}"#,
    r#"{"format": "json", "functions_unsupported_model": [2]}"#,
    r#"{"tool_choice": "auto"}"#,
    true
)]
#[case::function_call_alone_only_sets_the_format(
    r#"{"function_call": "auto"}"#,
    r#"{"format": "json"}"#,
    r#"{"function_call": "auto"}"#,
    true
)]
#[case::no_function_params_changes_nothing(r#"{"n": 1}"#, "{}", r#"{"n": 1}"#, false)]
fn function_fallback_for_ollama(
    #[case] non_default: &str,
    #[case] expected_optional: &str,
    #[case] expected_rest: &str,
    #[case] expected_prompt: bool,
) {
    let mut rest = params(non_default);
    let result = pre_process_optional_params(&params("{}"), &mut rest, "ollama");
    assert_eq!(result.optional_params, params(expected_optional));
    assert_eq!(rest, params(expected_rest));
    assert_eq!(result.add_function_to_prompt, expected_prompt);
}

#[rstest]
#[case::openai("openai")]
#[case::anthropic("anthropic")]
#[case::ollama_chat("ollama_chat")]
#[case::unknown("my_provider")]
fn function_fallback_leaves_other_providers_alone(#[case] provider: &str) {
    let original = params(r#"{"tools": [1], "functions": [2], "function_call": "auto"}"#);
    let mut rest = original.clone();
    let result = pre_process_optional_params(&params("{}"), &mut rest, provider);
    assert!(result.optional_params.is_empty());
    assert!(!result.add_function_to_prompt);
    assert_eq!(rest, original);
}

#[rstest]
#[case::azure("azure", r#"{"token": "t"}"#, r#"{"azure_ad_token": "t"}"#)]
#[case::bedrock("bedrock", r#"{"region_name": "r"}"#, r#"{"aws_region_name": "r"}"#)]
#[case::bedrock_ignores_token("bedrock", r#"{"token": "t"}"#, "{}")]
#[case::vertex_ai(
    "vertex_ai",
    r#"{"project": "p", "region_name": "r", "token": "t"}"#,
    r#"{"vertex_project": "p", "vertex_location": "r"}"#
)]
#[case::vertex_ai_beta(
    "vertex_ai_beta",
    r#"{"project": "p", "region_name": "r"}"#,
    r#"{"vertex_project": "p", "vertex_location": "r"}"#
)]
#[case::watsonx(
    "watsonx",
    r#"{"project": "p", "region_name": "r", "token": "t"}"#,
    r#"{"watsonx_project": "p", "watsonx_region_name": "r", "watsonx_token": "t"}"#
)]
#[case::openai_has_no_mapping("openai", r#"{"project": "p", "token": "t"}"#, "{}")]
fn function_fallback_maps_special_auth_params(
    #[case] provider: &str,
    #[case] passed: &str,
    #[case] expected: &str,
) {
    let result = pre_process_optional_params(&params(passed), &mut params("{}"), provider);
    assert_eq!(result.optional_params, params(expected));
}

#[test]
fn function_fallback_special_auth_keeps_a_null_value() {
    let result =
        pre_process_optional_params(&params(r#"{"token": null}"#), &mut params("{}"), "azure");
    assert_eq!(
        result.optional_params,
        params(r#"{"azure_ad_token": null}"#)
    );
}
