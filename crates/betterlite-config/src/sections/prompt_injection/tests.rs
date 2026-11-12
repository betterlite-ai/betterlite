#![cfg(test)]

use std::collections::BTreeMap;
use std::str::FromStr as _;
use std::time::Duration;

use rstest::rstest;
use rust_decimal::Decimal;

use crate::loader::{ConfigError, ConfigLoader};
use crate::sections::prompt_injection::{
    PromptInjectionParams, PromptInjectionParamsError, Settings,
};
use crate::types::Config;

const MAX_THREADS_VARIABLE: &str = "PROMPT_INJECTION_HEURISTICS_MAX_THREADS";
const THRESHOLD_VARIABLE: &str = "DEFAULT_PROMPT_INJECTION_SIMILARITY_THRESHOLD";
const TIMEOUT_VARIABLE: &str = "PROMPT_INJECTION_HEURISTICS_TIMEOUT_SECONDS";

fn environment(variables: &[(&str, &str)]) -> BTreeMap<String, String> {
    variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn load(source: &str, variables: &[(&str, &str)]) -> Result<Config, ConfigError> {
    ConfigLoader {
        lookup: environment(variables),
    }
    .load(source)
}

fn loaded(source: &str, variables: &[(&str, &str)]) -> Settings {
    load(source, variables).unwrap().sections.prompt_injection
}

fn load_error(source: &str, variables: &[(&str, &str)]) -> ConfigError {
    load(source, variables).unwrap_err()
}

fn default_params() -> PromptInjectionParams {
    PromptInjectionParams {
        heuristics_check: false,
        vector_db_check: false,
        llm_api_check: false,
        llm_api_name: None,
        llm_api_system_prompt: None,
        llm_api_fail_call_string: None,
        reject_as_response: Some(false),
    }
}

#[test]
fn a_missing_params_key_gives_none() {
    let settings = loaded("litellm_settings: {}", &[]);
    assert_eq!(settings.params, None);
    assert_eq!(settings.heuristics_max_threads, 1);
    assert_eq!(settings.heuristics_timeout, Duration::from_secs(30));
    assert_eq!(
        settings.similarity_threshold,
        Decimal::from_str("0.7").unwrap()
    );
}

#[test]
fn empty_params_give_the_litellm_defaults() {
    let settings = loaded("litellm_settings:\n  prompt_injection_params: {}\n", &[]);
    assert_eq!(settings.params, Some(default_params()));
}

#[test]
fn reads_every_param_key() {
    let settings = loaded(
        "litellm_settings:\n  prompt_injection_params:\n    heuristics_check: true\n    vector_db_check: true\n    llm_api_check: true\n    llm_api_name: judge\n    llm_api_system_prompt: system\n    llm_api_fail_call_string: UNSAFE\n    reject_as_response: true\n",
        &[],
    );
    assert_eq!(
        settings.params,
        Some(PromptInjectionParams {
            heuristics_check: true,
            vector_db_check: true,
            llm_api_check: true,
            llm_api_name: Some("judge".to_owned()),
            llm_api_system_prompt: Some("system".to_owned()),
            llm_api_fail_call_string: Some("UNSAFE".to_owned()),
            reject_as_response: Some(true),
        })
    );
}

#[test]
fn a_null_reject_as_response_is_none() {
    let settings = loaded(
        "litellm_settings:\n  prompt_injection_params:\n    reject_as_response: null\n",
        &[],
    );
    assert_eq!(
        settings.params.map(|params| params.reject_as_response),
        Some(None)
    );
}

#[test]
fn unknown_param_keys_are_ignored() {
    let settings = loaded(
        "litellm_settings:\n  prompt_injection_params:\n    heuristics_check: true\n    mystery: 1\n    nested:\n      deep: value\n",
        &[],
    );
    let expected = PromptInjectionParams {
        heuristics_check: true,
        vector_db_check: false,
        llm_api_check: false,
        llm_api_name: None,
        llm_api_system_prompt: None,
        llm_api_fail_call_string: None,
        reject_as_response: Some(false),
    };
    assert_eq!(settings.params, Some(expected));
}

#[rstest]
#[case::name_missing(
    "llm_api_check: true",
    "If llm_api_check is set to True, llm_api_name must be provided"
)]
#[case::name_empty(
    "llm_api_check: true\n    llm_api_name: ''",
    "If llm_api_check is set to True, llm_api_name must be provided"
)]
#[case::system_missing(
    "llm_api_check: true\n    llm_api_name: judge",
    "If llm_api_check is set to True, llm_api_system_prompt must be provided"
)]
#[case::system_empty(
    "llm_api_check: true\n    llm_api_name: judge\n    llm_api_system_prompt: ''",
    "If llm_api_check is set to True, llm_api_system_prompt must be provided"
)]
#[case::fail_string_missing(
    "llm_api_check: true\n    llm_api_name: judge\n    llm_api_system_prompt: system",
    "If llm_api_check is set to True, llm_api_fail_call_string must be provided"
)]
#[case::fail_string_empty(
    "llm_api_check: true\n    llm_api_name: judge\n    llm_api_system_prompt: system\n    llm_api_fail_call_string: ''",
    "If llm_api_check is set to True, llm_api_fail_call_string must be provided"
)]
fn incomplete_llm_api_params_fail_the_config(#[case] params: &str, #[case] message: &str) {
    let source = format!("litellm_settings:\n  prompt_injection_params:\n    {params}\n");
    let error = load_error(&source, &[]);
    assert_eq!(error.category(), "yaml");
    assert!(error.to_string().contains(message), "{error}");
}

#[rstest]
#[case::name(
    PromptInjectionParamsError::LlmApiNameMissing,
    "If llm_api_check is set to True, llm_api_name must be provided"
)]
#[case::system(
    PromptInjectionParamsError::LlmApiSystemPromptMissing,
    "If llm_api_check is set to True, llm_api_system_prompt must be provided"
)]
#[case::fail_string(
    PromptInjectionParamsError::LlmApiFailCallStringMissing,
    "If llm_api_check is set to True, llm_api_fail_call_string must be provided"
)]
fn validation_messages_match_litellm(
    #[case] error: PromptInjectionParamsError,
    #[case] message: &str,
) {
    assert_eq!(error.to_string(), message);
}

#[test]
fn empty_llm_api_values_load_when_the_check_is_off() {
    let settings = loaded(
        "litellm_settings:\n  prompt_injection_params:\n    llm_api_name: ''\n    llm_api_system_prompt: ''\n    llm_api_fail_call_string: ''\n",
        &[],
    );
    assert_eq!(
        settings.params.map(|params| params.llm_api_name),
        Some(Some(String::new()))
    );
}

#[rstest]
#[case::unset(&[], 1)]
#[case::empty(&[(MAX_THREADS_VARIABLE, "")], 1)]
#[case::four(&[(MAX_THREADS_VARIABLE, "4")], 4)]
#[case::padded(&[(MAX_THREADS_VARIABLE, " 2 ")], 2)]
#[case::zero(&[(MAX_THREADS_VARIABLE, "0")], 1)]
#[case::negative(&[(MAX_THREADS_VARIABLE, "-3")], 1)]
#[case::text(&[(MAX_THREADS_VARIABLE, "many")], 1)]
fn resolves_the_heuristics_max_threads(
    #[case] variables: &[(&str, &str)],
    #[case] expected: usize,
) {
    let settings = loaded("litellm_settings: {}", variables);
    assert_eq!(settings.heuristics_max_threads, expected);
}

#[rstest]
#[case::unset(&[], "0.7")]
#[case::plain(&[(THRESHOLD_VARIABLE, "0.85")], "0.85")]
#[case::scientific(&[(THRESHOLD_VARIABLE, "7e-1")], "0.7")]
#[case::trailing_zero(&[(THRESHOLD_VARIABLE, "0.70")], "0.70")]
#[case::padded(&[(THRESHOLD_VARIABLE, " 0.5 ")], "0.5")]
#[case::negative(&[(THRESHOLD_VARIABLE, "-1.5")], "-1.5")]
#[case::integer(&[(THRESHOLD_VARIABLE, "2")], "2")]
fn resolves_the_similarity_threshold(#[case] variables: &[(&str, &str)], #[case] expected: &str) {
    let settings = loaded("litellm_settings: {}", variables);
    assert_eq!(
        settings.similarity_threshold,
        Decimal::from_str(expected).unwrap()
    );
}

#[test]
fn an_empty_threshold_is_unset() {
    let settings = loaded("litellm_settings: {}", &[(THRESHOLD_VARIABLE, "")]);
    assert_eq!(
        settings.similarity_threshold,
        Decimal::from_str("0.7").unwrap()
    );
}

#[rstest]
#[case::text("not a number")]
#[case::nan("nan")]
#[case::inf("inf")]
#[case::negative_inf("-inf")]
fn an_unparsable_threshold_fails_the_config(#[case] value: &str) {
    let error = load_error("litellm_settings: {}", &[(THRESHOLD_VARIABLE, value)]);
    assert_eq!(error.category(), "environment");
    assert!(error.to_string().contains(THRESHOLD_VARIABLE), "{error}");
}

#[rstest]
#[case::unset(&[], Duration::from_secs(30))]
#[case::empty(&[(TIMEOUT_VARIABLE, "")], Duration::from_secs(30))]
#[case::plain(&[(TIMEOUT_VARIABLE, "45")], Duration::from_secs(45))]
#[case::fractional(&[(TIMEOUT_VARIABLE, "0.5")], Duration::from_millis(500))]
#[case::scientific(&[(TIMEOUT_VARIABLE, "1e1")], Duration::from_secs(10))]
fn resolves_the_heuristics_timeout(#[case] variables: &[(&str, &str)], #[case] expected: Duration) {
    let settings = loaded("litellm_settings: {}", variables);
    assert_eq!(settings.heuristics_timeout, expected);
}

#[rstest]
#[case::zero("0")]
#[case::negative("-5")]
#[case::text("soon")]
#[case::nan("nan")]
#[case::inf("inf")]
fn an_invalid_timeout_fails_the_config(#[case] value: &str) {
    let error = load_error("litellm_settings: {}", &[(TIMEOUT_VARIABLE, value)]);
    assert_eq!(error.category(), "environment");
    assert!(error.to_string().contains(TIMEOUT_VARIABLE), "{error}");
}
