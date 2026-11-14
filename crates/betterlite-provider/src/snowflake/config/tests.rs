#![cfg(test)]
use rstest::rstest;

use betterlite_prompts::arguments::json::{Json, Object};
use serde_json::Number;

use super::{
    get_complete_url, get_embed_complete_url, get_supported_openai_params, is_claude_model,
    map_embedding_openai_params, map_openai_params, provider_info, validate_environment,
};
use crate::api::file_contracts::config::Headers;
use crate::snowflake::SnowflakeError;

fn object(entries: &[(&str, Json)]) -> Object {
    Object::from_iter(
        entries
            .iter()
            .map(|(name, value)| ((*name).to_owned(), value.clone())),
    )
}

fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

#[rstest]
#[case::claude_dash("claude-3-5-sonnet", true)]
#[case::claude_underscore("claude_opus", true)]
#[case::prefixed("snowflake/claude-3-5-sonnet", true)]
#[case::capital_prefix("Snowflake/CLAUDE_x", true)]
#[case::no_separator("claude3", false)]
#[case::other("mistral-large", false)]
#[case::empty("", false)]
fn the_claude_test_reads_the_lowercased_name_without_the_provider_prefix(
    #[case] model: &str,
    #[case] expected: bool,
) {
    assert_eq!(is_claude_model(model), expected);
}

#[test]
fn chat_supported_params_keep_litellms_order_and_add_thinking_for_claude() {
    assert_eq!(
        get_supported_openai_params("snowflake/mistral-large"),
        vec![
            "temperature",
            "max_tokens",
            "max_completion_tokens",
            "top_p",
            "stream",
            "tools",
            "tool_choice"
        ]
    );
    assert_eq!(
        get_supported_openai_params("snowflake/claude-3-5-sonnet"),
        vec![
            "temperature",
            "max_tokens",
            "max_completion_tokens",
            "top_p",
            "stream",
            "tools",
            "tool_choice",
            "thinking"
        ]
    );
}

#[test]
fn map_openai_params_copies_supported_keys_and_ignores_the_rest() {
    let non_default = object(&[
        ("temperature", Json::Number(Number::from_f64(0.5).unwrap())),
        ("tool_choice", text("auto")),
        ("unsupported", text("dropped")),
        ("response_format", text("json_object")),
    ]);
    let mut optional = object(&[("top_p", Json::Number(Number::from_f64(0.9).unwrap()))]);
    map_openai_params(&non_default, &mut optional, "snowflake/mistral-large");
    assert_eq!(
        optional,
        object(&[
            ("top_p", Json::Number(Number::from_f64(0.9).unwrap())),
            ("temperature", Json::Number(Number::from_f64(0.5).unwrap())),
            ("tool_choice", text("auto")),
        ])
    );
}

#[test]
fn map_openai_params_keeps_response_format_for_claude_only_through_extra_keys() {
    let non_default = object(&[("response_format", text("json_object"))]);
    let mut optional = Object::new();
    map_openai_params(&non_default, &mut optional, "snowflake/claude-3-5-sonnet");
    assert!(optional.is_empty());
    map_embedding_openai_params(&non_default, &mut optional);
    assert_eq!(optional, non_default);
}

#[test]
fn a_given_api_base_is_trimmed_and_gets_the_version_suffix() {
    let mut optional = object(&[("account_id", text("left-in"))]);
    let base = get_complete_url(
        Some("https://example.test/custom///"),
        "snowflake/mistral-large",
        &mut optional,
        None,
    )
    .unwrap();
    assert_eq!(
        base,
        "https://example.test/custom/api/v2/cortex/v1/chat/completions"
    );
    assert_eq!(optional, object(&[("account_id", text("left-in"))]));
}

#[test]
fn a_given_api_base_that_already_ends_with_the_version_keeps_one_suffix() {
    let mut optional = Object::new();
    let base = get_complete_url(
        Some("https://example.test/api/v2/"),
        "snowflake/claude-3-5-sonnet",
        &mut optional,
        None,
    )
    .unwrap();
    assert_eq!(base, "https://example.test/api/v2/cortex/v1/messages");
}

#[test]
fn a_missing_api_base_is_built_from_the_account_id_in_optional_params() {
    let mut optional = object(&[("account_id", text("ab_c-1"))]);
    let base = get_embed_complete_url(None, &mut optional, Some("ignored")).unwrap();
    assert_eq!(
        base,
        "https://ab_c-1.snowflakecomputing.com/api/v2/cortex/inference:embed"
    );
    assert!(optional.is_empty());
}

#[test]
fn a_missing_api_base_falls_back_to_the_env_argument() {
    let mut optional = Object::new();
    let base = get_embed_complete_url(None, &mut optional, Some("env_id")).unwrap();
    assert_eq!(
        base,
        "https://env_id.snowflakecomputing.com/api/v2/cortex/inference:embed"
    );
}

#[test]
fn an_empty_api_base_counts_as_missing() {
    let mut optional = Object::new();
    let base = get_embed_complete_url(Some(""), &mut optional, Some("env_id")).unwrap();
    assert_eq!(
        base,
        "https://env_id.snowflakecomputing.com/api/v2/cortex/inference:embed"
    );
}

#[test]
fn the_account_id_from_optional_params_is_popped_even_when_it_is_null() {
    let mut optional = object(&[("account_id", Json::Null)]);
    assert_eq!(
        get_embed_complete_url(None, &mut optional, Some("env_id")),
        Err(SnowflakeError::MissingAccountId)
    );
    assert!(optional.is_empty());
}

#[rstest]
#[case::missing(None, SnowflakeError::MissingAccountId)]
#[case::empty(Some(""), SnowflakeError::InvalidAccountId)]
#[case::space(Some("bad id"), SnowflakeError::InvalidAccountId)]
#[case::dot(Some("bad.id"), SnowflakeError::InvalidAccountId)]
#[case::two_newlines(Some("good\n\n"), SnowflakeError::InvalidAccountId)]
#[case::newline_then_text(Some("good\nbad"), SnowflakeError::InvalidAccountId)]
fn an_account_id_outside_the_pattern_is_rejected(
    #[case] env: Option<&str>,
    #[case] expected: SnowflakeError,
) {
    let mut optional = Object::new();
    assert_eq!(
        get_embed_complete_url(None, &mut optional, env),
        Err(expected)
    );
}

#[test]
fn one_trailing_newline_after_a_valid_account_id_is_accepted_and_kept_in_the_host() {
    let mut optional = Object::new();
    let base = get_embed_complete_url(None, &mut optional, Some("good\n")).unwrap();
    assert_eq!(
        base,
        "https://good\n.snowflakecomputing.com/api/v2/cortex/inference:embed"
    );
}

#[test]
fn a_non_string_account_id_names_the_field() {
    let mut optional = object(&[("account_id", Json::Number(Number::from(5_i32)))]);
    assert_eq!(
        get_embed_complete_url(None, &mut optional, Some("env_id")),
        Err(SnowflakeError::AccountIdType)
    );
}

#[test]
fn a_missing_key_is_an_error_with_litellms_text() {
    let mut headers = Headers::new();
    let error = validate_environment(&mut headers, "snowflake/mistral-large", None).unwrap_err();
    assert_eq!(error.to_string(), "Missing Snowflake JWT key");
    assert!(headers.is_empty());
}

#[test]
fn a_jwt_key_sets_the_json_headers_the_bearer_value_and_the_keypair_type() {
    let mut headers = Headers::from_iter([("X-Kept".to_owned(), "value".to_owned())]);
    validate_environment(&mut headers, "snowflake/mistral-large", Some("jwt-token")).unwrap();
    assert_eq!(
        headers,
        Headers::from_iter([
            ("X-Kept".to_owned(), "value".to_owned()),
            ("Content-Type".to_owned(), "application/json".to_owned()),
            ("Accept".to_owned(), "application/json".to_owned()),
            ("Authorization".to_owned(), "Bearer jwt-token".to_owned()),
            (
                "X-Snowflake-Authorization-Token-Type".to_owned(),
                "KEYPAIR_JWT".to_owned()
            ),
        ])
    );
}

#[test]
fn a_pat_key_loses_the_prefix_and_sets_the_programmatic_type() {
    let mut headers = Headers::new();
    validate_environment(
        &mut headers,
        "snowflake/claude-3-5-sonnet",
        Some("pat/secret"),
    )
    .unwrap();
    assert_eq!(headers["Authorization"], "Bearer secret");
    assert_eq!(
        headers["X-Snowflake-Authorization-Token-Type"],
        "PROGRAMMATIC_ACCESS_TOKEN"
    );
    assert_eq!(headers["anthropic-version"], "2023-06-01");
}

#[test]
fn a_bare_pat_prefix_is_an_empty_token() {
    let mut headers = Headers::new();
    validate_environment(&mut headers, "snowflake/mistral-large", Some("pat/")).unwrap();
    assert_eq!(headers["Authorization"], "Bearer ");
    assert_eq!(
        headers["X-Snowflake-Authorization-Token-Type"],
        "PROGRAMMATIC_ACCESS_TOKEN"
    );
    assert!(!headers.contains_key("anthropic-version"));
}

#[test]
fn an_empty_key_is_not_none_and_stays_a_keypair_jwt() {
    let mut headers = Headers::new();
    validate_environment(&mut headers, "snowflake/mistral-large", Some("")).unwrap();
    assert_eq!(headers["Authorization"], "Bearer ");
    assert_eq!(
        headers["X-Snowflake-Authorization-Token-Type"],
        "KEYPAIR_JWT"
    );
}

#[test]
fn validate_environment_overwrites_the_headers_it_owns_and_keeps_the_rest() {
    let mut headers = Headers::from_iter([
        ("Accept".to_owned(), "text/plain".to_owned()),
        ("Authorization".to_owned(), "Bearer old".to_owned()),
        ("X-Kept".to_owned(), "value".to_owned()),
    ]);
    validate_environment(&mut headers, "snowflake/mistral-large", Some("new")).unwrap();
    assert_eq!(headers["Accept"], "application/json");
    assert_eq!(headers["Authorization"], "Bearer new");
    assert_eq!(headers["X-Kept"], "value");
    assert_eq!(headers.len(), 5);
}

#[test]
fn provider_info_keeps_the_base_and_prefers_a_non_empty_key() {
    assert_eq!(
        provider_info(Some("https://base"), Some("key"), Some("env-key")),
        (Some("https://base".to_owned()), Some("key".to_owned()))
    );
    assert_eq!(
        provider_info(None, Some(""), Some("env-key")),
        (None, Some("env-key".to_owned()))
    );
    assert_eq!(
        provider_info(None, None, Some("")),
        (None, Some(String::new()))
    );
    assert_eq!(provider_info(None, None, None), (None, None));
}
