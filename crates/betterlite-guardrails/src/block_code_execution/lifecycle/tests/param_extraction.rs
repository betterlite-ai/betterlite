use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::block_code_execution::initializer::{ParamKey, get_param};

use crate::block_code_execution::support::litellm_params;

#[test]
fn a_typed_mode_is_read_from_the_params() {
    let params = litellm_params(r#"{"guardrail": "block_code_execution", "mode": "pre_call"}"#);
    assert_eq!(
        get_param(&params, ParamKey::Mode),
        Some(GuardrailValue::Text("pre_call".to_owned()))
    );
}

#[test]
fn a_typed_list_mode_is_read_as_an_array() {
    let params = litellm_params(
        r#"{"guardrail": "block_code_execution", "mode": ["pre_call", "post_call"]}"#,
    );
    assert_eq!(
        get_param(&params, ParamKey::Mode),
        Some(GuardrailValue::Array(vec![
            GuardrailValue::Text("pre_call".to_owned()),
            GuardrailValue::Text("post_call".to_owned())
        ]))
    );
}

#[test]
fn a_typed_default_on_is_read_as_a_boolean() {
    let params = litellm_params(
        r#"{"guardrail": "block_code_execution", "mode": "pre_call", "default_on": true}"#,
    );
    assert_eq!(
        get_param(&params, ParamKey::DefaultOn),
        Some(GuardrailValue::Boolean(true))
    );
}

#[test]
fn an_untyped_key_falls_back_to_the_raw_mapping() {
    let params = litellm_params(
        r#"{"guardrail": "block_code_execution", "mode": "pre_call", "blocked_languages": ["python", "bash"], "confidence_threshold": 0.7}"#,
    );
    assert_eq!(
        get_param(&params, ParamKey::BlockedLanguages),
        Some(GuardrailValue::Array(vec![
            GuardrailValue::Text("python".to_owned()),
            GuardrailValue::Text("bash".to_owned())
        ]))
    );
    assert_eq!(
        get_param(&params, ParamKey::ConfidenceThreshold),
        Some(GuardrailValue::Decimal(0.7))
    );
}

#[test]
fn a_missing_key_reads_as_none() {
    let params = litellm_params(r#"{"guardrail": "block_code_execution", "mode": "pre_call"}"#);
    assert_eq!(get_param(&params, ParamKey::Action), None);
    assert_eq!(get_param(&params, ParamKey::DetectExecutionIntent), None);
}

#[test]
fn an_explicit_null_reads_as_null() {
    let params = litellm_params(
        r#"{"guardrail": "block_code_execution", "mode": "pre_call", "action": null}"#,
    );
    assert_eq!(
        get_param(&params, ParamKey::Action),
        Some(GuardrailValue::Null)
    );
}
