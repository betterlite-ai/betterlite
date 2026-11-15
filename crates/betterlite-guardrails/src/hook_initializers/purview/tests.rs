#![cfg(test)]

use betterlite_config::sections::guardrail_params::decode::decode_litellm_params;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_config::sections::guardrail_params::params::LitellmParams;
use rstest::rstest;
use secrecy::ExposeSecret as _;
use std::error::Error as _;

use super::initialize_purview;

fn params(json: &str) -> LitellmParams {
    decode_litellm_params(json.as_bytes()).unwrap()
}

const TENANT: &str = r#""guardrail": "microsoft_purview", "mode": "pre_call",
    "tenant_id": "tenant", "client_id": "client""#;

#[test]
fn purview_reads_the_named_fields_from_the_params_only() {
    let args = initialize_purview(
        &params(&format!(
            r#"{{{TENANT}, "api_key": "secret", "timeout": 6}}"#
        )),
        Some("purview-check"),
    )
    .unwrap();
    assert_eq!(args.guardrail_name, "purview-check");
    assert_eq!(args.tenant_id, "tenant");
    assert_eq!(args.client_id, "client");
    assert_eq!(args.client_secret.expose_secret(), "secret");
    assert_eq!(args.purview_app_name, "LiteLLM");
    assert_eq!(args.user_id_field, "user_id");
    assert_eq!(args.timeout, Some(6.0_f64));
    assert_eq!(
        args.event_hook,
        LitellmParamsMode::One("pre_call".to_owned())
    );
    assert!(!args.default_on);
}

#[test]
fn purview_falls_back_from_api_key_to_client_secret() {
    let from_param = initialize_purview(
        &params(&format!(
            r#"{{{TENANT}, "client_secret": "client-secret"}}"#
        )),
        Some("name"),
    )
    .unwrap();
    assert_eq!(from_param.client_secret.expose_secret(), "client-secret");
    let api_key_wins = initialize_purview(
        &params(&format!(
            r#"{{{TENANT}, "api_key": "from-api-key", "client_secret": "ignored"}}"#
        )),
        Some("name"),
    )
    .unwrap();
    assert_eq!(api_key_wins.client_secret.expose_secret(), "from-api-key");
    let empty_api_key = initialize_purview(
        &params(&format!(
            r#"{{{TENANT}, "api_key": "", "client_secret": "client-secret"}}"#
        )),
        Some("name"),
    )
    .unwrap();
    assert_eq!(empty_api_key.client_secret.expose_secret(), "client-secret");
}

#[test]
fn purview_defaults_the_app_name_and_user_field_for_absent_or_empty_values() {
    let args = initialize_purview(
        &params(&format!(
            r#"{{{TENANT}, "api_key": "secret", "purview_app_name": "",
                "user_id_field": ""}}"#
        )),
        Some("name"),
    )
    .unwrap();
    assert_eq!(args.purview_app_name, "LiteLLM");
    assert_eq!(args.user_id_field, "user_id");
    let named = initialize_purview(
        &params(&format!(
            r#"{{{TENANT}, "api_key": "secret", "purview_app_name": "Gateway",
                "user_id_field": "email"}}"#
        )),
        Some("name"),
    )
    .unwrap();
    assert_eq!(named.purview_app_name, "Gateway");
    assert_eq!(named.user_id_field, "email");
}

#[test]
fn purview_turns_numeric_app_names_into_text() {
    let args = initialize_purview(
        &params(&format!(
            r#"{{{TENANT}, "api_key": "secret", "purview_app_name": 2024, "user_id_field": 7}}"#
        )),
        Some("name"),
    )
    .unwrap();
    assert_eq!(args.purview_app_name, "2024");
    assert_eq!(args.user_id_field, "7");
    let decimal = initialize_purview(
        &params(&format!(
            r#"{{{TENANT}, "api_key": "secret", "purview_app_name": 2.5}}"#
        )),
        Some("name"),
    )
    .unwrap();
    assert_eq!(decimal.purview_app_name, "2.5");
    let huge = initialize_purview(
        &params(&format!(
            r#"{{{TENANT}, "api_key": "secret", "user_id_field": 18446744073709551615}}"#
        )),
        Some("name"),
    )
    .unwrap();
    assert_eq!(huge.user_id_field, "18446744073709551615");
}

#[rstest]
#[case::no_tenant(
    r#"{"guardrail": "microsoft_purview", "mode": "pre_call"}"#,
    Some("name"),
    "Microsoft Purview: tenant_id is required"
)]
#[case::no_client(
    r#"{"guardrail": "microsoft_purview", "mode": "pre_call", "tenant_id": "tenant"}"#,
    Some("name"),
    "Microsoft Purview: client_id is required"
)]
#[case::no_secret(
    r#"{"guardrail": "microsoft_purview", "mode": "pre_call", "tenant_id": "tenant", "client_id": "client"}"#,
    Some("name"),
    "Microsoft Purview: client_secret (or api_key) is required"
)]
#[case::empty_secret(
    r#"{"guardrail": "microsoft_purview", "mode": "pre_call", "tenant_id": "tenant", "client_id": "client", "client_secret": ""}"#,
    Some("name"),
    "Microsoft Purview: client_secret (or api_key) is required"
)]
#[case::credentials_before_name(
    r#"{"guardrail": "microsoft_purview", "mode": "pre_call"}"#,
    None,
    "Microsoft Purview: tenant_id is required"
)]
fn purview_refuses_missing_values_in_litellm_order(
    #[case] json: &str,
    #[case] name: Option<&str>,
    #[case] message: &str,
) {
    let error = initialize_purview(&params(json), name).unwrap_err();
    assert_eq!(error.to_string(), message);
}

#[test]
fn purview_refuses_a_missing_guardrail_name() {
    let complete = params(&format!(r#"{{{TENANT}, "api_key": "secret"}}"#));
    let unnamed = initialize_purview(&complete, None).unwrap_err();
    assert_eq!(
        unnamed.to_string(),
        "Microsoft Purview: guardrail_name is required"
    );
}

#[test]
fn purview_refuses_a_boolean_app_name_and_keeps_the_decoder_cause() {
    let error = initialize_purview(
        &params(&format!(
            r#"{{{TENANT}, "api_key": "secret", "purview_app_name": true}}"#
        )),
        Some("name"),
    )
    .unwrap_err();
    assert_eq!(
        error.to_string(),
        "Microsoft Purview: purview_app_name has an invalid value"
    );
    assert!(error.source().is_some());
}
