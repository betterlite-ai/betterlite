#![cfg(test)]

use betterlite_config::sections::guardrail_params::base::UnreachableFallback;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use rstest::rstest;

use super::initialize_akto;
use crate::third_party_init_a::error::ThirdPartyInitAError;
use crate::third_party_init_a::tests::{exposed, params};

#[test]
fn akto_takes_the_defaults_of_an_empty_config() {
    let built = initialize_akto(&params(""), None).unwrap();
    assert_eq!(built.name, "");
    assert_eq!(built.akto_base_url, None);
    assert!(built.akto_api_key.is_none());
    assert_eq!(built.akto_account_id, None);
    assert_eq!(built.akto_vxlan_id, None);
    assert_eq!(built.unreachable_fallback, UnreachableFallback::FailClosed);
    assert_eq!(built.guardrail_timeout, None);
    assert!(!built.default_on);
    assert_eq!(
        built.event_hook,
        LitellmParamsMode::One("pre_call".to_owned())
    );
}

#[test]
fn akto_copies_the_configured_fields() {
    let built = initialize_akto(
        &params(
            r#",
            "akto_base_url": "https://akto.test",
            "akto_api_key": "akto-key",
            "akto_account_id": "1234",
            "akto_vxlan_id": "7",
            "unreachable_fallback": "fail_open",
            "guardrail_timeout": 9,
            "default_on": true
            "#,
        ),
        Some("akto-guard"),
    )
    .unwrap();
    assert_eq!(built.name, "akto-guard");
    assert_eq!(built.akto_base_url.as_deref(), Some("https://akto.test"));
    assert_eq!(exposed(built.akto_api_key.as_ref()), Some("akto-key"));
    assert_eq!(built.akto_account_id.as_deref(), Some("1234"));
    assert_eq!(built.akto_vxlan_id.as_deref(), Some("7"));
    assert_eq!(built.unreachable_fallback, UnreachableFallback::FailOpen);
    assert_eq!(built.guardrail_timeout, Some(9));
    assert!(built.default_on);
}

#[test]
fn akto_reads_the_timeout_from_guardrail_timeout_only() {
    let built = initialize_akto(&params(r#", "timeout": 30"#), None).unwrap();
    assert_eq!(built.guardrail_timeout, None);
}

#[test]
fn akto_reads_a_null_param_as_unset() {
    let built = initialize_akto(
        &params(r#", "akto_base_url": null, "guardrail_timeout": null"#),
        None,
    )
    .unwrap();
    assert_eq!(built.akto_base_url, None);
    assert_eq!(built.guardrail_timeout, None);
}

#[rstest]
#[case::timeout_text(r#", "guardrail_timeout": "slow""#, "guardrail_timeout")]
#[case::timeout_fraction(r#", "guardrail_timeout": 1.5"#, "guardrail_timeout")]
#[case::url_number(r#", "akto_base_url": 5"#, "akto_base_url")]
#[case::vxlan_number(r#", "akto_vxlan_id": 5"#, "akto_vxlan_id")]
fn akto_names_the_param_of_a_wrong_type(#[case] extras: &str, #[case] field: &str) {
    let error = initialize_akto(&params(extras), None).unwrap_err();
    assert!(matches!(
        &error,
        ThirdPartyInitAError::InvalidParam { name } if name == field
    ));
}
