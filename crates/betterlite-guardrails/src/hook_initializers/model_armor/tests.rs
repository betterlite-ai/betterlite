#![cfg(test)]

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::decode::decode_litellm_params;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_config::sections::guardrail_params::params::LitellmParams;
use secrecy::ExposeSecret as _;

use super::initialize_model_armor;

fn params(json: &str) -> LitellmParams {
    decode_litellm_params(json.as_bytes()).unwrap()
}

fn configured() -> LitellmParams {
    params(
        r#"{"guardrail": "model_armor", "mode": "during_call", "template_id": "template", "project_id": "project", "location": "europe-west4", "credentials": "/secrets/credentials.json", "api_endpoint": "https://modelarmor.example", "default_on": true, "mask_request_content": true, "mask_response_content": false, "fail_on_error": false, "skip_unscannable_attachments": true, "sanitize_error_detail": false, "timeout": 4.5}"#,
    )
}

#[test]
fn model_armor_copies_the_identifying_fields() {
    let args = initialize_model_armor(&configured(), Some("armor-check"));
    assert_eq!(args.guardrail_name, "armor-check");
    assert_eq!(args.template_id.as_deref(), Some("template"));
    assert_eq!(
        args.project_id,
        Some(GuardrailValue::Text("project".to_owned()))
    );
    assert_eq!(args.location.as_deref(), Some("europe-west4"));
    assert_eq!(
        args.credentials.as_ref().unwrap().expose_secret(),
        "/secrets/credentials.json"
    );
    assert_eq!(
        args.api_endpoint.as_deref(),
        Some("https://modelarmor.example")
    );
}

#[test]
fn model_armor_copies_the_flags_timeout_and_mode() {
    let args = initialize_model_armor(&configured(), Some("armor-check"));
    assert!(args.default_on);
    assert_eq!(args.mask_request_content, Some(true));
    assert_eq!(args.mask_response_content, Some(false));
    assert!(!args.fail_on_error);
    assert!(args.skip_unscannable_attachments);
    assert!(!args.sanitize_error_detail);
    assert_eq!(args.timeout, Some(4.5_f64));
    assert_eq!(
        args.event_hook,
        LitellmParamsMode::One("during_call".to_owned())
    );
}

#[test]
fn model_armor_uses_the_empty_name_and_the_param_defaults() {
    let args = initialize_model_armor(
        &params(r#"{"guardrail": "model_armor", "mode": "pre_call"}"#),
        None,
    );
    assert_eq!(args.guardrail_name, "");
    assert_eq!(args.template_id, None);
    assert_eq!(args.project_id, None);
    assert_eq!(args.location, None);
    assert!(args.credentials.is_none());
    assert_eq!(args.api_endpoint, None);
    assert!(!args.default_on);
    assert_eq!(args.mask_request_content, None);
    assert_eq!(args.mask_response_content, None);
    assert!(args.fail_on_error);
    assert!(!args.skip_unscannable_attachments);
    assert!(args.sanitize_error_detail);
    assert_eq!(args.timeout, None);
}

#[test]
fn model_armor_copies_the_project_id_unchanged() {
    let args = initialize_model_armor(
        &params(r#"{"guardrail": "model_armor", "mode": "pre_call", "project_id": 7}"#),
        Some("name"),
    );
    assert_eq!(args.project_id, Some(GuardrailValue::SignedInteger(7)));
    let absent = initialize_model_armor(
        &params(r#"{"guardrail": "model_armor", "mode": "pre_call", "project_id": null}"#),
        Some("name"),
    );
    assert_eq!(absent.project_id, None);
}
