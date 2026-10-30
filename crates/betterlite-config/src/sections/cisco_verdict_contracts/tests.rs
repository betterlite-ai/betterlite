#![cfg(test)]

use std::collections::BTreeMap;

use secrecy::ExposeSecret as _;

use crate::loader::parse_view;
use crate::sections::Section as _;

#[test]
fn cisco_config_loads_through_loader() {
    // Parses only this section's own view, the way RawSections::parse does per section.
    let (raw, _ignored) = parse_view::<super::Raw>(
        "guardrails:\n  - guardrail_name: cisco\n    litellm_params:\n      guardrail: cisco_ai_defense\n      api_key: test-key\n",
    )
    .expect("Cisco section YAML must parse");
    let settings = raw
        .convert(&BTreeMap::new())
        .expect("Cisco configuration must load through the public loader");
    let configured = settings.guardrails.first().expect("Cisco settings");
    assert_eq!(configured.api_key.expose_secret(), "test-key");
    assert_eq!(configured.inspect_path, "/api/v1/inspect/chat");
}

#[test]
fn cisco_config_loads_secret_and_defaults() {
    let entry = super::Entry {
        guardrail_name: Some("cisco".to_owned()),
        litellm_params: super::Parameters {
            guardrail: "cisco_ai_defense".to_owned(),
            api_key: None,
            api_base: None,
            inspection_type: None,
            inspect_path: None,
            enabled_rules: Vec::new(),
            on_flagged_action: None,
            fallback_on_error: None,
            timeout: None,
            integration_profile_id: None,
            integration_profile_version: None,
            integration_tenant_id: None,
            integration_type: None,
        },
    };
    let configured = entry
        .resolve(
            0,
            &BTreeMap::from([(
                String::from("CISCO_AI_DEFENSE_API_KEY"),
                String::from("secret"),
            )]),
        )
        .expect("config");
    assert_eq!(configured.api_key.expose_secret(), "secret");
    assert_eq!(configured.timeout.to_bits(), 10.0_f64.to_bits());
    assert_eq!(configured.inspect_path, "/api/v1/inspect/chat");
}

#[test]
fn cisco_config_accepts_bare_string_rules() {
    let (raw, _ignored) = parse_view::<super::Raw>(
        "guardrails:\n  - guardrail_name: cisco\n    litellm_params:\n      guardrail: cisco_ai_defense\n      api_key: test-key\n      enabled_rules:\n        - Prompt Injection\n        - rule_name: PII\n          entity_types: [Email Address]\n",
    )
    .expect("Cisco section YAML must parse");
    let settings = raw
        .convert(&BTreeMap::new())
        .expect("Cisco configuration must load through the public loader");
    let configured = settings.guardrails.first().expect("Cisco settings");
    assert_eq!(configured.enabled_rules.len(), 2);
    assert_eq!(configured.enabled_rules[0].rule_name, "Prompt Injection");
    assert!(configured.enabled_rules[0].entity_types.is_empty());
    assert_eq!(configured.enabled_rules[1].rule_name, "PII");
    assert_eq!(configured.enabled_rules[1].entity_types, ["Email Address"]);
}
