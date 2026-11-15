#![cfg(test)]

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::decode::decode_litellm_params;
use betterlite_config::sections::guardrail_params::params::LitellmParams;
use rstest::rstest;
use secrecy::ExposeSecret as _;

use super::{AzureArgs, AzureCommon, initialize_azure};

fn params(json: &str) -> LitellmParams {
    decode_litellm_params(json.as_bytes()).unwrap()
}

fn prompt_shield(args: AzureArgs) -> AzureCommon {
    match args {
        AzureArgs::PromptShield(common) => common,
        AzureArgs::TextModerations(_) => panic!("expected the prompt shield variant"),
    }
}

#[test]
fn azure_picks_the_variant_from_the_text_after_the_first_slash() {
    let shield = params(
        r#"{"guardrail": "azure/prompt_shield", "mode": "pre_call", "api_key": "key", "api_base": "https://azure.example"}"#,
    );
    assert!(matches!(
        initialize_azure(&shield, Some("name")).unwrap(),
        AzureArgs::PromptShield(_)
    ));
    let moderation = params(
        r#"{"guardrail": "azure/text_moderations", "mode": "pre_call", "api_key": "key", "api_base": "https://azure.example"}"#,
    );
    assert!(matches!(
        initialize_azure(&moderation, Some("name")).unwrap(),
        AzureArgs::TextModerations(_)
    ));
    let trailing = params(
        r#"{"guardrail": "azure/prompt_shield/extra", "mode": "pre_call", "api_key": "key", "api_base": "https://azure.example"}"#,
    );
    assert!(matches!(
        initialize_azure(&trailing, Some("name")).unwrap(),
        AzureArgs::PromptShield(_)
    ));
}

#[rstest]
#[case::no_key(
    r#"{"guardrail": "azure/prompt_shield", "mode": "pre_call"}"#,
    Some("name"),
    "Azure Content Safety: api_key is required"
)]
#[case::empty_key(
    r#"{"guardrail": "azure/prompt_shield", "mode": "pre_call", "api_key": ""}"#,
    Some("name"),
    "Azure Content Safety: api_key is required"
)]
#[case::no_base(
    r#"{"guardrail": "azure/prompt_shield", "mode": "pre_call", "api_key": "key"}"#,
    Some("name"),
    "Azure Content Safety: api_base is required"
)]
#[case::empty_base(
    r#"{"guardrail": "azure/prompt_shield", "mode": "pre_call", "api_key": "key", "api_base": ""}"#,
    Some("name"),
    "Azure Content Safety: api_base is required"
)]
#[case::no_name(
    r#"{"guardrail": "azure/prompt_shield", "mode": "pre_call", "api_key": "key", "api_base": "https://azure.example"}"#,
    None,
    "Azure Content Safety: guardrail_name is required"
)]
#[case::missing_key_without_name(
    r#"{"guardrail": "azure/prompt_shield", "mode": "pre_call"}"#,
    None,
    "Azure Content Safety: api_key is required"
)]
#[case::unknown_part(
    r#"{"guardrail": "azure/other", "mode": "pre_call", "api_key": "key", "api_base": "https://azure.example"}"#,
    Some("name"),
    "Azure Content Safety: other is not a valid guardrail"
)]
#[case::no_slash(
    r#"{"guardrail": "azure", "mode": "pre_call", "api_key": "key", "api_base": "https://azure.example"}"#,
    Some("name"),
    "Azure Content Safety: azure is not a valid guardrail"
)]
#[case::bare_part(
    r#"{"guardrail": "prompt_shield", "mode": "pre_call", "api_key": "key", "api_base": "https://azure.example"}"#,
    Some("name"),
    "Azure Content Safety: prompt_shield is not a valid guardrail"
)]
#[case::unknown_part_without_name(
    r#"{"guardrail": "azure/other", "mode": "pre_call", "api_key": "key", "api_base": "https://azure.example"}"#,
    None,
    "Azure Content Safety: guardrail_name is required"
)]
fn azure_refuses_missing_values_in_litellm_order(
    #[case] json: &str,
    #[case] name: Option<&str>,
    #[case] message: &str,
) {
    let error = initialize_azure(&params(json), name).unwrap_err();
    assert_eq!(error.to_string(), message);
}

#[test]
fn azure_extras_keep_the_non_null_params_and_explicit_fields_win() {
    let common = prompt_shield(
        initialize_azure(
            &params(
                r#"{"guardrail": "azure/prompt_shield", "mode": "pre_call", "api_key": "azure-key", "api_base": "https://azure.example", "default_on": true, "timeout": 3, "vendor_flag": true, "null_extra": null}"#,
            ),
            Some("azure-check"),
        )
        .unwrap(),
    );
    assert_eq!(common.api_key.expose_secret(), "azure-key");
    let extras = &common.extras.0;
    assert_eq!(
        extras.get("guardrail"),
        Some(&GuardrailValue::Text("azure/prompt_shield".to_owned()))
    );
    assert_eq!(
        extras.get("api_key"),
        Some(&GuardrailValue::Text("azure-key".to_owned()))
    );
    assert_eq!(
        extras.get("default_on"),
        Some(&GuardrailValue::Boolean(true))
    );
    assert_eq!(
        extras.get("event_hook"),
        Some(&GuardrailValue::Text("pre_call".to_owned()))
    );
    assert_eq!(
        extras.get("fail_on_error"),
        Some(&GuardrailValue::Boolean(true))
    );
    assert_eq!(
        extras.get("vendor_flag"),
        Some(&GuardrailValue::Boolean(true))
    );
    assert!(!extras.contains_key("null_extra"));
    assert!(!extras.values().any(|value| *value == GuardrailValue::Null));
}

#[test]
fn azure_keeps_the_params_when_default_on_is_absent() {
    let common = prompt_shield(
        initialize_azure(
            &params(
                r#"{"guardrail": "azure/prompt_shield", "mode": "pre_call", "api_key": "key", "api_base": "https://azure.example"}"#,
            ),
            Some("name"),
        )
        .unwrap(),
    );
    assert!(!common.default_on);
    assert_eq!(
        common.extras.0.get("default_on"),
        Some(&GuardrailValue::Boolean(false))
    );
    assert!(!common.extras.0.contains_key("model"));
    assert!(!common.extras.0.contains_key("timeout"));
}
