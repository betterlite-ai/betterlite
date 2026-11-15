#![cfg(test)]

use betterlite_config::sections::cisco_verdict_contracts::{FallbackAction, InspectionType};
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use rstest::rstest;

use super::{CiscoRule, CiscoRuleEntry, initialize_cisco};
use crate::third_party_init_a::error::ThirdPartyInitAError;
use crate::third_party_init_a::tests::{exposed, params};

#[rstest]
#[case::missing(None)]
#[case::empty(Some(""))]
fn cisco_refuses_a_missing_or_empty_name(#[case] name: Option<&str>) {
    let error = initialize_cisco(&params(""), name).unwrap_err();
    assert!(matches!(error, ThirdPartyInitAError::MissingCiscoName));
    assert_eq!(
        error.to_string(),
        "Cisco AI Defense: guardrail_name is required"
    );
}

#[test]
fn cisco_takes_the_defaults_of_an_empty_config() {
    let built = initialize_cisco(&params(""), Some("cisco-guard")).unwrap();
    assert_eq!(built.name, "cisco-guard");
    assert!(built.api_key.is_none());
    assert_eq!(built.api_base, None);
    assert_eq!(built.inspection_type, None);
    assert_eq!(built.inspect_path, None);
    assert_eq!(built.enabled_rules, None);
    assert_eq!(built.integration_profile_id, None);
    assert_eq!(built.integration_profile_version, None);
    assert_eq!(built.integration_tenant_id, None);
    assert_eq!(built.integration_type, None);
    assert_eq!(built.on_flagged_action, None);
    assert_eq!(built.fallback_on_error, None);
    assert_eq!(built.timeout, None);
    assert!(!built.default_on);
    assert_eq!(
        built.event_hook,
        LitellmParamsMode::One("pre_call".to_owned())
    );
}

#[test]
fn cisco_copies_the_plain_params_and_the_optional_params() {
    let built = initialize_cisco(
        &params(
            r#",
            "api_key": "cisco-key",
            "api_base": "https://cisco.test",
            "default_on": true,
            "optional_params": {
                "inspection_type": "mcp",
                "inspect_path": "/custom/path",
                "enabled_rules": ["PII", {"rule_name": "PCI", "entity_types": ["Card"]}],
                "integration_profile_id": "profile-1",
                "integration_profile_version": "2",
                "integration_tenant_id": "tenant-3",
                "integration_type": "gateway",
                "on_flagged_action": "monitor",
                "fallback_on_error": "allow",
                "timeout": 20
            }
            "#,
        ),
        Some("cisco-guard"),
    )
    .unwrap();
    assert_eq!(exposed(built.api_key.as_ref()), Some("cisco-key"));
    assert_eq!(built.api_base.as_deref(), Some("https://cisco.test"));
    assert!(built.default_on);
    assert_eq!(built.inspection_type, Some(InspectionType::Mcp));
    assert_eq!(built.inspect_path.as_deref(), Some("/custom/path"));
    assert_eq!(
        built.enabled_rules,
        Some(vec![
            CiscoRule::Name("PII".to_owned()),
            CiscoRule::Entry(CiscoRuleEntry {
                rule_name: Some("PCI".to_owned()),
                entity_types: Some(vec!["Card".to_owned()]),
                rule_id: None,
                classification: None,
            }),
        ])
    );
    assert_eq!(built.integration_profile_id.as_deref(), Some("profile-1"));
    assert_eq!(built.integration_profile_version.as_deref(), Some("2"));
    assert_eq!(built.integration_tenant_id.as_deref(), Some("tenant-3"));
    assert_eq!(built.integration_type.as_deref(), Some("gateway"));
    assert_eq!(built.on_flagged_action.as_deref(), Some("monitor"));
    assert_eq!(built.fallback_on_error, Some(FallbackAction::Allow));
    assert_eq!(built.timeout, Some(20.0_f64));
}

#[test]
fn cisco_reads_a_top_level_param_the_config_set() {
    let built = initialize_cisco(
        &params(
            r#",
            "inspection_type": "chat",
            "inspect_path": "/top/path",
            "enabled_rules": ["Harassment"],
            "integration_profile_id": "top-profile",
            "integration_profile_version": "9",
            "integration_tenant_id": "top-tenant",
            "integration_type": "proxy",
            "on_flagged_action": "block",
            "fallback_on_error": "block",
            "timeout": 7
            "#,
        ),
        Some("cisco-guard"),
    )
    .unwrap();
    assert_eq!(built.inspection_type, Some(InspectionType::Chat));
    assert_eq!(built.inspect_path.as_deref(), Some("/top/path"));
    assert_eq!(
        built.enabled_rules,
        Some(vec![CiscoRule::Name("Harassment".to_owned())])
    );
    assert_eq!(built.integration_profile_id.as_deref(), Some("top-profile"));
    assert_eq!(built.integration_profile_version.as_deref(), Some("9"));
    assert_eq!(built.integration_tenant_id.as_deref(), Some("top-tenant"));
    assert_eq!(built.integration_type.as_deref(), Some("proxy"));
    assert_eq!(built.on_flagged_action.as_deref(), Some("block"));
    assert_eq!(built.fallback_on_error, Some(FallbackAction::Block));
    assert_eq!(built.timeout, Some(7.0_f64));
}

#[test]
fn cisco_lets_an_optional_params_value_win_over_the_top_level() {
    let built = initialize_cisco(
        &params(
            r#",
            "inspection_type": "chat",
            "timeout": 7,
            "optional_params": {"inspection_type": "mcp", "timeout": 30}
            "#,
        ),
        Some("cisco-guard"),
    )
    .unwrap();
    assert_eq!(built.inspection_type, Some(InspectionType::Mcp));
    assert_eq!(built.timeout, Some(30.0_f64));
}

#[test]
fn cisco_falls_back_to_the_top_level_for_a_null_optional_entry() {
    let built = initialize_cisco(
        &params(
            r#",
            "inspect_path": "/top/path",
            "timeout": 7,
            "optional_params": {"inspect_path": null, "timeout": null}
            "#,
        ),
        Some("cisco-guard"),
    )
    .unwrap();
    assert_eq!(built.inspect_path.as_deref(), Some("/top/path"));
    assert_eq!(built.timeout, Some(7.0_f64));
}

#[rstest]
#[case::null_entry_and_nothing_at_top_level(r#", "optional_params": {"inspect_path": null}"#)]
#[case::top_level_null(r#", "inspect_path": null"#)]
#[case::null_on_both_sides(r#", "inspect_path": null, "optional_params": {"inspect_path": null}"#)]
#[case::other_optional_entry(r#", "optional_params": {"inspection_type": "chat"}"#)]
fn cisco_leaves_an_unset_param_unset(#[case] extras: &str) {
    let built = initialize_cisco(&params(extras), Some("cisco-guard")).unwrap();
    assert_eq!(built.inspect_path, None);
}

#[test]
fn cisco_does_not_take_a_sibling_default_for_an_unset_param() {
    let built = initialize_cisco(
        &params(r#", "optional_params": {"inspect_path": "/x"}"#),
        Some("cisco-guard"),
    )
    .unwrap();
    assert_eq!(built.on_flagged_action, None);
    assert_eq!(built.fallback_on_error, None);
    assert_eq!(built.timeout, None);
    assert_eq!(built.inspection_type, None);
}

#[rstest]
#[case::inspection_type_unknown(
    r#", "optional_params": {"inspection_type": "other"}"#,
    "inspection_type"
)]
#[case::fallback_unknown(r#", "fallback_on_error": "maybe""#, "fallback_on_error")]
#[case::timeout_text(r#", "optional_params": {"timeout": "slow"}"#, "timeout")]
#[case::path_number(r#", "optional_params": {"inspect_path": 5}"#, "inspect_path")]
#[case::rules_text(r#", "optional_params": {"enabled_rules": "PII"}"#, "enabled_rules")]
#[case::profile_number(r#", "integration_profile_id": 3"#, "integration_profile_id")]
#[case::optional_params_text(r#", "optional_params": "text""#, "optional_params")]
fn cisco_names_the_param_of_a_wrong_type(#[case] extras: &str, #[case] field: &str) {
    let error = initialize_cisco(&params(extras), Some("cisco-guard")).unwrap_err();
    assert!(matches!(
        &error,
        ThirdPartyInitAError::InvalidParam { name } if name == field
    ));
}
