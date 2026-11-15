#![cfg(test)]

use betterlite_config::sections::guardrail_params::base::UnreachableFallback;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;

use super::initialize_deepkeep;
use crate::third_party_init_a::error::ThirdPartyInitAError;
use crate::third_party_init_a::tests::{exposed, params};

#[test]
fn deepkeep_takes_the_defaults_of_an_empty_config() {
    let built = initialize_deepkeep(&params(""), None).unwrap();
    assert_eq!(built.name, "");
    assert_eq!(built.api_base, None);
    assert!(built.api_key.is_none());
    assert_eq!(built.firewall_id, None);
    assert_eq!(built.unreachable_fallback, UnreachableFallback::FailClosed);
    assert_eq!(built.extra_headers, None);
    assert_eq!(built.timeout, None);
    assert!(!built.default_on);
    assert_eq!(
        built.event_hook,
        LitellmParamsMode::One("pre_call".to_owned())
    );
}

#[test]
fn deepkeep_copies_the_configured_fields() {
    let built = initialize_deepkeep(
        &params(
            r#",
            "api_base": "https://deepkeep.test",
            "api_key": "deepkeep-key",
            "deepkeep_firewall_id": "firewall-1",
            "unreachable_fallback": "fail_open",
            "extra_headers": ["x-request-id", "x-team"],
            "timeout": 6,
            "default_on": true
            "#,
        ),
        Some("deepkeep-guard"),
    )
    .unwrap();
    assert_eq!(built.name, "deepkeep-guard");
    assert_eq!(built.api_base.as_deref(), Some("https://deepkeep.test"));
    assert_eq!(exposed(built.api_key.as_ref()), Some("deepkeep-key"));
    assert_eq!(built.firewall_id.as_deref(), Some("firewall-1"));
    assert_eq!(built.unreachable_fallback, UnreachableFallback::FailOpen);
    assert_eq!(
        built.extra_headers,
        Some(vec!["x-request-id".to_owned(), "x-team".to_owned()])
    );
    assert_eq!(built.timeout, Some(6.0_f64));
    assert!(built.default_on);
}

#[test]
fn deepkeep_reads_the_firewall_id_from_deepkeep_firewall_id_only() {
    let built = initialize_deepkeep(&params(r#", "firewall_id": "wrong""#), None).unwrap();
    assert_eq!(built.firewall_id, None);
}

#[test]
fn deepkeep_names_a_firewall_id_of_a_wrong_type() {
    let error = initialize_deepkeep(&params(r#", "deepkeep_firewall_id": 3"#), None).unwrap_err();
    assert!(matches!(
        &error,
        ThirdPartyInitAError::InvalidParam { name } if name == "deepkeep_firewall_id"
    ));
}
