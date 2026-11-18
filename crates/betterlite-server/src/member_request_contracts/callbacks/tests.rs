#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use rstest::rstest;

use super::{AddTeamCallback, CallbackSecrets, CallbackType, TeamCallbackMetadata};
use crate::member_request_contracts::MemberContractError;

fn parse(text: &str) -> Json {
    serde_json::from_str(text).unwrap()
}

fn callback(body: &str) -> Result<AddTeamCallback, MemberContractError> {
    AddTeamCallback::from_value(&parse(body))
}

fn metadata(body: &str) -> Result<TeamCallbackMetadata, MemberContractError> {
    TeamCallbackMetadata::from_value(&parse(body))
}

fn message(error: MemberContractError) -> String {
    match error {
        MemberContractError::Validation { message } => message,
        MemberContractError::Decode { source } => {
            panic!("expected a validation error, got {source}")
        }
    }
}

#[test]
fn defaults_the_callback_type_to_success_and_failure() {
    let request = callback(r#"{"callback_name": "langfuse", "callback_vars": {}}"#).unwrap();
    assert_eq!(request.callback_type, Some(CallbackType::SuccessAndFailure));
    assert_eq!(request.callback_name, "langfuse");
    assert_eq!(request.callback_vars, CallbackSecrets::default());
}

#[test]
fn keeps_an_explicit_null_callback_type() {
    let request = callback(
        r#"{
            "callback_name": "langfuse",
            "callback_type": null,
            "callback_vars": {}
        }"#,
    )
    .unwrap();
    assert_eq!(request.callback_type, None);
}

#[rstest]
#[case("success", CallbackType::Success)]
#[case("failure", CallbackType::Failure)]
#[case("success_and_failure", CallbackType::SuccessAndFailure)]
fn decodes_each_callback_type(#[case] text: &str, #[case] expected: CallbackType) {
    let body = format!(
        r#"{{"callback_name": "langfuse", "callback_type": "{text}", "callback_vars": {{}}}}"#
    );
    let request = callback(&body).unwrap();
    assert_eq!(request.callback_type, Some(expected));
}

#[test]
fn rejects_an_unknown_callback_type() {
    let error = callback(
        r#"{
            "callback_name": "langfuse",
            "callback_type": "on_error",
            "callback_vars": {}
        }"#,
    )
    .unwrap_err();
    assert!(matches!(error, MemberContractError::Decode { .. }));
}

#[test]
fn rejects_a_body_without_a_callback_name() {
    let error = callback(r#"{"callback_vars": {}}"#).unwrap_err();
    assert!(matches!(error, MemberContractError::Decode { .. }));
}

#[test]
fn rejects_a_body_without_callback_vars() {
    let error = callback(r#"{"callback_name": "langfuse"}"#).unwrap_err();
    assert!(matches!(error, MemberContractError::Decode { .. }));
}

#[test]
fn rejects_a_null_callback_vars() {
    let error = callback(r#"{"callback_name": "langfuse", "callback_vars": null}"#).unwrap_err();
    assert_eq!(message(error), "Input should be a valid dictionary");
}

#[test]
fn turns_number_callback_vars_into_their_text() {
    let request = callback(
        r#"{
            "callback_name": "langfuse",
            "callback_vars": { "langfuse_prompt_version": 2, "langfuse_host": "https://langfuse" }
        }"#,
    )
    .unwrap();
    let expected = BTreeMap::from([
        ("langfuse_prompt_version".to_owned(), "2".to_owned()),
        ("langfuse_host".to_owned(), "https://langfuse".to_owned()),
    ]);
    assert_eq!(request.callback_vars, CallbackSecrets::from(expected));
}

#[test]
fn keeps_callback_credentials_out_of_debug() {
    let request = callback(
        r#"{
            "callback_name": "langsmith",
            "callback_vars": { "langsmith_api_key": "ls-secret-value" }
        }"#,
    )
    .unwrap();
    let debug = format!("{request:?}");
    assert!(!debug.contains("ls-secret-value"), "{debug}");
    assert!(debug.contains("[REDACTED]"), "{debug}");
    let fields = metadata(
        r#"{
            "callback_vars": { "langsmith_api_key": "ls-secret-value" }
        }"#,
    )
    .unwrap();
    let metadata_debug = format!("{fields:?}");
    assert!(
        !metadata_debug.contains("ls-secret-value"),
        "{metadata_debug}"
    );
    assert!(metadata_debug.contains("[REDACTED]"), "{metadata_debug}");
}

#[test]
fn serializes_callback_credentials_in_plain_text() {
    let request = callback(
        r#"{
            "callback_name": "langsmith",
            "callback_vars": { "langsmith_api_key": "ls-secret-value" }
        }"#,
    )
    .unwrap();
    let text = serde_json::to_string(&request).unwrap();
    assert!(
        text.contains(r#""langsmith_api_key":"ls-secret-value""#),
        "{text}"
    );
    let fields = metadata(
        r#"{
            "callback_vars": { "langsmith_api_key": "ls-secret-value" }
        }"#,
    )
    .unwrap();
    let metadata_text = serde_json::to_string(&fields).unwrap();
    assert!(
        metadata_text.contains(r#""langsmith_api_key":"ls-secret-value""#),
        "{metadata_text}"
    );
}

#[test]
fn tells_callback_secrets_apart() {
    let host = CallbackSecrets::from(BTreeMap::from([(
        "langfuse_host".to_owned(),
        "same-value".to_owned(),
    )]));
    assert_eq!(host, host.clone());
    let other_value = CallbackSecrets::from(BTreeMap::from([(
        "langfuse_host".to_owned(),
        "other-value".to_owned(),
    )]));
    assert_ne!(host, other_value);
    let other_name = CallbackSecrets::from(BTreeMap::from([(
        "langsmith_base_url".to_owned(),
        "same-value".to_owned(),
    )]));
    assert_ne!(host, other_name);
}

#[rstest]
#[case("true")]
#[case("[\"az\"]")]
#[case("{\"az\": 1}")]
fn rejects_a_callback_var_value_that_is_not_text(#[case] value: &str) {
    let body = format!(
        r#"{{"callback_name": "langfuse", "callback_vars": {{"langfuse_host": {value}}}}}"#
    );
    let error = callback(&body).unwrap_err();
    assert!(matches!(error, MemberContractError::Decode { .. }));
}

#[test]
fn rejects_an_unknown_callback_variable() {
    let error = callback(
        r#"{
            "callback_name": "langfuse",
            "callback_vars": { "bogus_host": "https://langfuse" }
        }"#,
    )
    .unwrap_err();
    let text = message(error);
    assert!(
        text.starts_with("Invalid callback variable: bogus_host. Must be one of ["),
        "{text}"
    );
    assert!(
        text.contains("'arize_api_key', 'arize_error_sampling_rate'"),
        "{text}"
    );
    assert!(
        text.ends_with("'wandb_api_key', 'weave_project_id']"),
        "{text}"
    );
}

#[test]
fn names_the_first_invalid_callback_variable_in_the_order_sent() {
    let error = callback(
        r#"{"callback_name":"langfuse","callback_vars":{"zulu_bad":"1","alpha_bad":"2"}}"#,
    )
    .unwrap_err();
    let text = message(error);
    assert!(
        text.starts_with("Invalid callback variable: zulu_bad."),
        "{text}"
    );
}

#[test]
fn rejects_an_environment_reference() {
    let error = callback(
        r#"{
            "callback_name": "langfuse",
            "callback_vars": { "langfuse_host": "os.environ/LANGFUSE_HOST" }
        }"#,
    )
    .unwrap_err();
    let expected = concat!(
        "Callback param 'langfuse_host' (from key/team callback metadata) contains an ",
        "'os.environ/' reference. Environment references in request-supplied parameters ",
        "are no longer resolved server-side for security reasons.\n",
        "To resolve:\n",
        "  1. Remove the 'os.environ/' reference from your request body / metadata.\n",
        "  2. Either (a) configure this callback value in your proxy config.yaml under ",
        "'litellm_settings' / 'general_settings', or (b) pass the resolved secret value ",
        "directly in the request.\n",
        "See https://docs.litellm.ai/docs/proxy/logging for server-side callback ",
        "configuration.",
    );
    assert_eq!(message(error), expected);
}

#[rstest]
#[case("langfuse_environment", "production")]
#[case("langfuse_environment", "prod-1")]
#[case("langfuse_environment", "prod_eu")]
#[case("langfuse_span_scope", "full")]
#[case("langfuse_span_scope", "llm_only")]
fn keeps_valid_langfuse_values(#[case] name: &str, #[case] value: &str) {
    let expected = CallbackSecrets::from(BTreeMap::from([(name.to_owned(), value.to_owned())]));
    let body =
        format!(r#"{{"callback_name": "langfuse", "callback_vars": {{"{name}": "{value}"}}}}"#);
    let request = callback(&body).unwrap();
    assert_eq!(request.callback_vars, expected, "{name} {value}");
}

#[rstest]
#[case("Production")]
#[case("langfuse-prod")]
#[case("prod eu")]
fn rejects_an_invalid_langfuse_environment(#[case] environment: &str) {
    let body = format!(
        r#"{{"callback_name": "langfuse", "callback_vars": {{"langfuse_environment": "{environment}"}}}}"#
    );
    let error = callback(&body).unwrap_err();
    let expected = format!(
        "Invalid langfuse_environment '{environment}': must be lowercase \
         alphanumerics/hyphens/underscores and must not start with 'langfuse' \
         (pattern ^(?!langfuse)[a-z0-9-_]+$)"
    );
    assert_eq!(message(error), expected);
}

#[test]
fn rejects_a_trailing_newline_in_the_environment() {
    let error = callback(
        r#"{
            "callback_name": "langfuse",
            "callback_vars": { "langfuse_environment": "production\n" }
        }"#,
    )
    .unwrap_err();
    assert!(message(error).starts_with("Invalid langfuse_environment 'production\n'"));
}

#[test]
fn rejects_an_invalid_langfuse_span_scope() {
    let error = callback(
        r#"{
            "callback_name": "langfuse",
            "callback_vars": { "langfuse_span_scope": "traces" }
        }"#,
    )
    .unwrap_err();
    let expected =
        "Invalid langfuse_span_scope 'traces': must be one of ['full', 'llm_only']".to_owned();
    assert_eq!(message(error), expected);
}

#[test]
fn gives_the_metadata_defaults_for_an_empty_object() {
    let fields = metadata("{}").unwrap();
    assert_eq!(fields, TeamCallbackMetadata::default());
}

#[test]
fn gives_the_metadata_defaults_when_every_key_is_null() {
    let fields = metadata(
        r#"{
            "success_callback": null,
            "failure_callback": null,
            "callbacks": null,
            "callback_vars": null
        }"#,
    )
    .unwrap();
    assert_eq!(fields, TeamCallbackMetadata::default());
}

#[test]
fn gives_the_metadata_defaults_when_only_unknown_null_keys_remain() {
    let fields = metadata(r#"{"callbacks": null, "unknown": null}"#).unwrap();
    assert_eq!(fields, TeamCallbackMetadata::default());
}

#[test]
fn drops_null_keys_and_keeps_the_others() {
    let fields = metadata(
        r#"{
            "success_callback": null,
            "failure_callback": ["slack"],
            "callbacks": ["langfuse"]
        }"#,
    )
    .unwrap();
    assert!(fields.success_callback.is_empty());
    assert_eq!(fields.failure_callback, vec!["slack"]);
    assert_eq!(fields.callbacks, vec!["langfuse"]);
    assert_eq!(fields.callback_vars, CallbackSecrets::default());
}

#[test]
fn ignores_an_unknown_key_that_is_not_null() {
    let fields = metadata(
        r#"{
            "callbacks": ["langfuse"],
            "callback_vars": {},
            "unknown": 1
        }"#,
    )
    .unwrap();
    assert_eq!(fields.callbacks, vec!["langfuse"]);
    assert_eq!(fields.callback_vars, CallbackSecrets::default());
}

#[test]
fn rejects_an_unknown_metadata_callback_variable() {
    let error = metadata(
        r#"{
            "callbacks": ["langfuse"],
            "callback_vars": { "bogus_host": "https://langfuse" }
        }"#,
    )
    .unwrap_err();
    let text = message(error);
    assert!(
        text.starts_with("Invalid callback variable: bogus_host. Must be one of ["),
        "{text}"
    );
}

#[test]
fn accepts_callback_var_values_the_request_would_check() {
    let fields = metadata(
        r#"{
            "callback_vars": { "langfuse_host": "os.environ/LANGFUSE_HOST" }
        }"#,
    )
    .unwrap();
    let expected = BTreeMap::from([(
        "langfuse_host".to_owned(),
        "os.environ/LANGFUSE_HOST".to_owned(),
    )]);
    assert_eq!(fields.callback_vars, CallbackSecrets::from(expected));
}

#[test]
fn rejects_metadata_callback_vars_that_is_not_an_object() {
    let error = metadata(
        r#"{
            "callbacks": ["langfuse"],
            "callback_vars": ["langfuse_host"]
        }"#,
    )
    .unwrap_err();
    assert!(matches!(error, MemberContractError::Decode { .. }));
}

#[test]
fn rejects_a_callback_body_that_is_not_an_object() {
    let callback_error = callback("\"langfuse\"").unwrap_err();
    assert!(matches!(
        callback_error,
        MemberContractError::Validation { .. }
    ));
    let metadata_error = metadata("[]").unwrap_err();
    assert!(matches!(
        metadata_error,
        MemberContractError::Validation { .. }
    ));
}
