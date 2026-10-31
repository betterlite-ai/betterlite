#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::sync::Arc;

use rstest::rstest;
use secrecy::ExposeSecret as _;
use tracing::subscriber::with_default;

use crate::loader::{ConfigError, ConfigLoader, parse_view};
use crate::sections::Section as _;

#[test]
fn cisco_config_loads_through_loader() {
    let loader = ConfigLoader {
        lookup: BTreeMap::new(),
    };
    let config = loader
        .load(
            "guardrails:\n  - guardrail_name: cisco\n    litellm_params:\n      guardrail: cisco_ai_defense\n      api_key: test-key\n",
        )
        .expect("Cisco configuration must load through the public loader");
    let configured = config
        .sections
        .cisco_verdict_contracts
        .guardrails
        .first()
        .expect("Cisco settings");
    assert_eq!(configured.api_key.expose_secret(), "test-key");
    assert_eq!(configured.inspect_path, "/api/v1/inspect/chat");
}

#[rstest]
#[case::unset(None)]
#[case::empty(Some(""))]
fn cisco_config_requires_api_key(#[case] value: Option<&str>) {
    let loader = ConfigLoader {
        lookup: value
            .into_iter()
            .map(|text| ("CISCO_AI_DEFENSE_API_KEY".to_owned(), text.to_owned()))
            .collect::<BTreeMap<_, _>>(),
    };
    let error = loader.load("guardrails:\n  - guardrail_name: cisco\n    litellm_params:\n      guardrail: cisco_ai_defense\n").unwrap_err();
    assert!(
        matches!(error, ConfigError::MissingEnvironment { variable, path }
        if variable == "CISCO_AI_DEFENSE_API_KEY" && path == "guardrails.0.litellm_params.api_key")
    );
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
            enabled_rules: None,
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
    assert_eq!(
        configured.api_base,
        "https://us.api.inspect.aidefense.security.cisco.com"
    );
    assert_eq!(configured.inspection_type, super::InspectionType::Chat);
    assert_eq!(configured.on_flagged_action, super::FlaggedAction::Block);
    assert_eq!(configured.fallback_on_error, super::FallbackAction::Block);
    assert!(!format!("{configured:?}").contains("secret"));
}

#[test]
fn cisco_config_loads_mcp_and_integration() {
    let loader = ConfigLoader {
        lookup: [
            ("TEST_KEY", "confidential-value"),
            ("CISCO_AI_DEFENSE_API_BASE", "https://inspect.example.test"),
            ("CISCO_AI_DEFENSE_INSPECTION_TYPE", "mcp"),
            ("CISCO_AI_DEFENSE_ON_FLAGGED_ACTION", "monitor"),
            ("CISCO_AI_DEFENSE_FALLBACK_ON_ERROR", "allow"),
        ]
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value.to_owned()))
        .collect::<BTreeMap<_, _>>(),
    };
    let config = loader.load("guardrails:\n  - guardrail_name: cisco\n    litellm_params:\n      guardrail: cisco_ai_defense\n      api_key: os.environ/TEST_KEY\n      integration_profile_id: profile\n      integration_profile_version: v2\n      integration_tenant_id: tenant\n      integration_type: application\n").unwrap();
    let configured = config
        .sections
        .cisco_verdict_contracts
        .guardrails
        .first()
        .unwrap();
    assert_eq!(configured.api_key.expose_secret(), "confidential-value");
    assert_eq!(configured.api_base, "https://inspect.example.test");
    assert_eq!(configured.inspection_type, super::InspectionType::Mcp);
    assert_eq!(configured.inspect_path, "/api/v1/inspect/mcp");
    assert_eq!(configured.on_flagged_action, super::FlaggedAction::Monitor);
    assert_eq!(configured.fallback_on_error, super::FallbackAction::Allow);
    assert_eq!(
        configured.integration_profile_id.as_deref(),
        Some("profile")
    );
    assert_eq!(
        configured.integration_profile_version.as_deref(),
        Some("v2")
    );
    assert_eq!(configured.integration_tenant_id.as_deref(), Some("tenant"));
    assert_eq!(configured.integration_type.as_deref(), Some("application"));
}

#[rstest]
#[case::below("0", 1.0_f64, true)]
#[case::minimum("1", 1.0_f64, false)]
#[case::maximum("60", 60.0_f64, false)]
#[case::above("61", 60.0_f64, true)]
fn cisco_config_clamps_with_warning(
    #[case] input: &str,
    #[case] expected: f64,
    #[case] warns: bool,
) {
    let log = tempfile::NamedTempFile::new().unwrap();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_writer(Arc::new(log.reopen().unwrap()))
        .finish();
    let loader = ConfigLoader {
        lookup: BTreeMap::from([
            ("CISCO_AI_DEFENSE_API_KEY".to_owned(), "test-key".to_owned()),
            ("CISCO_AI_DEFENSE_TIMEOUT".to_owned(), input.to_owned()),
        ]),
    };
    let config = with_default(subscriber, || loader.load("guardrails:\n  - guardrail_name: cisco\n    litellm_params:\n      guardrail: cisco_ai_defense\n      inspect_path: /custom\n")).unwrap();
    let configured = config
        .sections
        .cisco_verdict_contracts
        .guardrails
        .first()
        .unwrap();
    assert_eq!(configured.timeout.to_bits(), expected.to_bits());
    assert_eq!(configured.inspect_path, "/custom");
    let output = fs::read_to_string(log.path()).unwrap();
    assert_eq!(output.contains("WARN"), warns, "{output}");
    assert_eq!(output.contains("clamping"), warns, "{output}");
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

#[test]
fn cisco_config_preserves_rule_id_and_classification() {
    let (raw, _ignored) = parse_view::<super::Raw>(
        "guardrails:\n  - guardrail_name: cisco\n    litellm_params:\n      guardrail: cisco_ai_defense\n      api_key: test-key\n      enabled_rules:\n        - rule_name: PII\n          rule_id: rule-123\n          classification: sensitive\n          entity_types: null\n",
    )
    .expect("Cisco section YAML must parse");
    let settings = raw
        .convert(&BTreeMap::new())
        .expect("Cisco configuration must load through the public loader");
    let configured = settings.guardrails.first().expect("Cisco settings");
    let rule = configured.enabled_rules.first().expect("rule");
    assert_eq!(rule.rule_id.as_deref(), Some("rule-123"));
    assert_eq!(rule.classification.as_deref(), Some("sensitive"));
    assert!(rule.entity_types.is_empty());
}

#[test]
fn cisco_config_rejects_unsupported_rule_fields() {
    let result = parse_view::<super::Raw>(
        "guardrails:\n  - guardrail_name: cisco\n    litellm_params:\n      guardrail: cisco_ai_defense\n      api_key: test-key\n      enabled_rules:\n        - rule_name: PII\n          bogus_field: oops\n",
    );
    assert!(
        result.is_err(),
        "an unsupported rule field must be rejected, not silently dropped"
    );
}

#[test]
fn cisco_config_accepts_null_enabled_rules() {
    let (raw, _ignored) = parse_view::<super::Raw>(
        "guardrails:\n  - guardrail_name: cisco\n    litellm_params:\n      guardrail: cisco_ai_defense\n      api_key: test-key\n      enabled_rules: null\n",
    )
    .expect("Cisco section YAML must parse");
    let settings = raw
        .convert(&BTreeMap::new())
        .expect("Cisco configuration must load through the public loader");
    let configured = settings.guardrails.first().expect("Cisco settings");
    assert!(configured.enabled_rules.is_empty());
}

#[test]
fn cisco_config_coerces_quoted_timeout() {
    let (raw, _ignored) = parse_view::<super::Raw>(
        "guardrails:\n  - guardrail_name: cisco\n    litellm_params:\n      guardrail: cisco_ai_defense\n      api_key: test-key\n      timeout: \"20\"\n",
    )
    .expect("Cisco section YAML must parse");
    let settings = raw
        .convert(&BTreeMap::new())
        .expect("Cisco configuration must load through the public loader");
    let configured = settings.guardrails.first().expect("Cisco settings");
    assert_eq!(configured.timeout.to_bits(), 20.0_f64.to_bits());
}

#[test]
fn cisco_config_defaults_invalid_on_flagged_action() {
    let entry = super::Entry {
        guardrail_name: Some("cisco".to_owned()),
        litellm_params: super::Parameters {
            guardrail: "cisco_ai_defense".to_owned(),
            api_key: None,
            api_base: None,
            inspection_type: None,
            inspect_path: None,
            enabled_rules: None,
            on_flagged_action: Some("nonsense".to_owned()),
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
        .expect("an invalid on_flagged_action must warn and default, not fail config loading");
    assert_eq!(configured.on_flagged_action, super::FlaggedAction::Block);
}
