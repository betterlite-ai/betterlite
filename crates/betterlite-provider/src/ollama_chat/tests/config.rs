use std::collections::BTreeMap;

use betterlite_openai::error::ProviderError;
use betterlite_prompts::arguments::json::Json;
use indexmap::IndexMap;
use rstest::rstest;
use secrecy::SecretString;

use super::super::config::{
    OllamaChatConfig, SUPPORTED_OPENAI_PARAMS, complete_url, map_openai_params,
    validate_environment,
};
use super::super::error::OllamaError;

fn object(json: &str) -> IndexMap<String, Json> {
    serde_json::from_str(json).unwrap()
}

fn mapped(non_default: &str, model: &str) -> Result<IndexMap<String, Json>, OllamaError> {
    let mut optional = IndexMap::new();
    map_openai_params(&object(non_default), &mut optional, model)?;
    Ok(optional)
}

#[test]
fn supported_params_are_litellms_list() {
    assert_eq!(
        SUPPORTED_OPENAI_PARAMS,
        [
            "max_tokens",
            "max_completion_tokens",
            "stream",
            "top_p",
            "temperature",
            "seed",
            "frequency_penalty",
            "stop",
            "tools",
            "tool_choice",
            "functions",
            "response_format",
            "reasoning_effort",
        ]
    );
}

#[rstest]
#[case::max_tokens(r#"{"max_tokens": 5}"#, r#"{"num_predict": 5}"#)]
#[case::max_completion_tokens(r#"{"max_completion_tokens": 7}"#, r#"{"num_predict": 7}"#)]
#[case::max_completion_tokens_wins(
    r#"{"max_tokens": 5, "max_completion_tokens": 7}"#,
    r#"{"num_predict": 7}"#
)]
#[case::stream_true(r#"{"stream": true}"#, r#"{"stream": true}"#)]
#[case::stream_false(r#"{"stream": false}"#, r#"{"stream": false}"#)]
#[case::temperature(r#"{"temperature": 0}"#, r#"{"temperature": 0}"#)]
#[case::seed(r#"{"seed": 1}"#, r#"{"seed": 1}"#)]
#[case::top_p(r#"{"top_p": 0.9}"#, r#"{"top_p": 0.9}"#)]
#[case::frequency_penalty(r#"{"frequency_penalty": 1.2}"#, r#"{"repeat_penalty": 1.2}"#)]
#[case::stop(r#"{"stop": ["x", "y"]}"#, r#"{"stop": ["x", "y"]}"#)]
#[case::json_object(
    r#"{"response_format": {"type": "json_object"}}"#,
    r#"{"format": "json"}"#
)]
#[case::json_schema(
    r#"{"response_format": {"type": "json_schema", "json_schema": {"schema": {"type": "object"}}}}"#,
    r#"{"format": {"type": "object"}}"#
)]
#[case::json_schema_empty_is_used_as_given(
    r#"{"response_format": {"type": "json_schema", "json_schema": {"schema": {}}}}"#,
    r#"{"format": {}}"#
)]
#[case::json_schema_without_schema(r#"{"response_format": {"type": "json_schema"}}"#, "{}")]
#[case::json_schema_null_schema(
    r#"{"response_format": {"type": "json_schema", "json_schema": {"schema": null}}}"#,
    "{}"
)]
#[case::response_format_text(r#"{"response_format": {"type": "text"}}"#, "{}")]
#[case::response_format_text_ignores_a_json_schema(
    r#"{"response_format": {"type": "text", "json_schema": {"schema": {"type": "object"}}}}"#,
    "{}"
)]
#[case::response_format_scalar(r#"{"response_format": "json"}"#, "{}")]
#[case::reasoning_low(r#"{"reasoning_effort": "low"}"#, r#"{"think": true}"#)]
#[case::reasoning_high(r#"{"reasoning_effort": "high"}"#, r#"{"think": true}"#)]
#[case::reasoning_medium(r#"{"reasoning_effort": "medium"}"#, r#"{"think": true}"#)]
#[case::reasoning_minimal(r#"{"reasoning_effort": "minimal"}"#, r#"{"think": false}"#)]
#[case::reasoning_null(r#"{"reasoning_effort": null}"#, "{}")]
#[case::tools(
    r#"{"tools": [{"type": "function", "function": {"name": "f"}}]}"#,
    r#"{"tools": [{"type": "function", "function": {"name": "f"}}]}"#
)]
#[case::functions_become_tools(
    r#"{"functions": [{"name": "f"}]}"#,
    r#"{"tools": [{"name": "f"}]}"#
)]
#[case::tool_choice_is_never_mapped(r#"{"tool_choice": "auto"}"#, "{}")]
#[case::unknown_params_are_ignored(
    r#"{"max_tokens": 5, "temperature": 0, "top_p": 1, "other": 3}"#,
    r#"{"num_predict": 5, "temperature": 0, "top_p": 1}"#
)]
fn map_openai_params_matches_litellm(#[case] non_default: &str, #[case] expected: &str) {
    assert_eq!(mapped(non_default, "llama3").unwrap(), object(expected));
}

#[test]
fn reasoning_effort_is_passed_through_for_a_gpt_oss_model() {
    assert_eq!(
        mapped(r#"{"reasoning_effort": "medium"}"#, "gpt-oss:20b").unwrap(),
        object(r#"{"think": "medium"}"#)
    );
}

#[test]
fn map_openai_params_overwrites_an_existing_option() {
    let mut optional = object(r#"{"num_predict": 9}"#);
    map_openai_params(&object(r#"{"max_tokens": 5}"#), &mut optional, "llama3").unwrap();
    assert_eq!(optional, object(r#"{"num_predict": 5}"#));
}

#[rstest]
#[case::json_schema_scalar(r#"{"response_format": {"type": "json_schema", "json_schema": "x"}}"#)]
#[case::schema_scalar(
    r#"{"response_format": {"type": "json_schema", "json_schema": {"schema": "x"}}}"#
)]
#[case::reasoning_effort_number(r#"{"reasoning_effort": 2}"#)]
fn a_parameter_of_the_wrong_type_is_a_client_error(#[case] non_default: &str) {
    let error = mapped(non_default, "llama3").unwrap_err();
    assert_eq!(error.status(), Some(400));
}

#[test]
fn get_config_is_empty_by_default() {
    assert!(OllamaChatConfig::default().get_config().is_empty());
}

#[test]
fn get_config_keeps_litellms_order_and_values() {
    let config: OllamaChatConfig = serde_json::from_str(
        r#"{"seed": 0, "mirostat": 1, "temperature": 0.5, "system": "", "stop": ["x"], "top_p": 0.9}"#,
    )
    .unwrap();
    let found = config.get_config();
    assert_eq!(
        found.keys().map(String::as_str).collect::<Vec<_>>(),
        ["mirostat", "seed", "system", "temperature", "stop", "top_p"]
    );
    assert_eq!(
        found,
        object(
            r#"{"mirostat": 1, "seed": 0, "system": "", "temperature": 0.5, "stop": ["x"], "top_p": 0.9}"#
        )
    );
}

#[test]
fn get_config_carries_every_option() {
    let config: OllamaChatConfig = serde_json::from_str(
        r#"{"mirostat": 0, "mirostat_eta": 0.1, "mirostat_tau": 5.0, "num_ctx": 4096, "num_gqa": 8,
            "num_thread": 8, "repeat_last_n": 64, "repeat_penalty": 1.1, "seed": 42, "tfs_z": 1,
            "num_predict": 42, "top_k": 40, "system": "s", "template": "t"}"#,
    )
    .unwrap();
    assert_eq!(config.get_config().len(), 14);
}

#[test]
fn an_unknown_config_key_is_rejected() {
    serde_json::from_str::<OllamaChatConfig>(r#"{"nope": 1}"#).unwrap_err();
}

#[test]
fn validate_environment_adds_the_bearer_header_only_with_a_key() {
    let mut headers = BTreeMap::new();
    validate_environment(&mut headers, None);
    assert!(headers.is_empty());
    validate_environment(&mut headers, Some(&SecretString::from("")));
    assert_eq!(
        headers,
        BTreeMap::from([(String::from("Authorization"), String::from("Bearer "))])
    );
    validate_environment(&mut headers, Some(&SecretString::from("sk-x")));
    assert_eq!(
        headers.get("Authorization").map(String::as_str),
        Some("Bearer ")
    );
}

#[test]
fn validate_environment_respects_an_existing_authorization_key_case() {
    let mut exact = BTreeMap::from([(String::from("Authorization"), String::from("Bearer other"))]);
    validate_environment(&mut exact, Some(&SecretString::from("sk-x")));
    assert_eq!(exact.len(), 1);

    let mut lower = BTreeMap::from([(String::from("authorization"), String::from("Bearer other"))]);
    validate_environment(&mut lower, Some(&SecretString::from("sk-x")));
    assert_eq!(
        lower,
        BTreeMap::from([
            (String::from("Authorization"), String::from("Bearer sk-x")),
            (String::from("authorization"), String::from("Bearer other")),
        ])
    );
}

#[rstest]
#[case::missing(None, "http://localhost:11434/api/chat")]
#[case::plain(Some("http://h:11434"), "http://h:11434/api/chat")]
#[case::trailing_slash(Some("http://h:11434/"), "http://h:11434//api/chat")]
#[case::chat_path(Some("http://h/api/chat"), "http://h/api/chat")]
#[case::chat_path_with_slash(Some("http://h/api/chat/"), "http://h/api/chat//api/chat")]
fn complete_url_appends_the_chat_path(#[case] api_base: Option<&str>, #[case] expected: &str) {
    assert_eq!(complete_url(api_base), expected);
}

#[test]
fn the_api_error_carries_litellms_status_and_headers() {
    let error = OllamaError::Api {
        status_code: 418,
        message: String::from("boom"),
        headers: BTreeMap::from([(String::from("x"), String::from("1"))]),
    };
    assert_eq!(error.status(), Some(418));
    let converted = ProviderError::from(error);
    assert_eq!(converted.status_code, 418);
    assert_eq!(converted.message, "boom");
    assert_eq!(
        converted.headers,
        Some(vec![(String::from("x"), String::from("1"))])
    );
}

#[test]
fn a_missing_chunk_field_is_a_client_error() {
    let error = OllamaError::ChunkMissing { field: "message" };
    assert_eq!(error.status(), Some(400));
}

#[rstest]
#[case::reply_field("reply_field")]
#[case::chunk_field("chunk_field")]
fn a_wrong_typed_value_has_no_status(#[case] kind: &str) {
    let error = match kind {
        "reply_field" => OllamaError::ReplyField { field: "x" },
        _ => OllamaError::ChunkField { field: "x" },
    };
    assert_eq!(error.status(), None);
    assert_eq!(ProviderError::from(error).status_code, 500);
}
