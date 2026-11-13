#![cfg(test)]

use std::collections::BTreeMap;

use secrecy::ExposeSecret as _;

use crate::loader::{ConfigError, ConfigLoader};

#[test]
fn panw_prisma_airs_configuration_loads_defaults_and_secret() {
    let loader = ConfigLoader {
        lookup: BTreeMap::new(),
    };
    let config = loader.load("guardrails:\n  - guardrail_name: prisma\n    litellm_params:\n      guardrail: panw_prisma_airs\n      api_key: secret-key\n").expect("config");
    let configured = config
        .sections
        .panw_prisma_airs
        .guardrails
        .first()
        .expect("guardrail");
    assert_eq!(configured.api_key.expose_secret(), "secret-key");
    assert_eq!(
        configured.api_base,
        "https://service.api.aisecurity.paloaltonetworks.com"
    );
    assert_eq!(configured.app_name, "LiteLLM");
    assert!(!configured.masking.content.request_content);
    assert!(!configured.masking.content.response_content);
    assert_eq!(configured.timeout.to_bits(), 10.0_f64.to_bits());
    assert!(configured.default_on);
    assert!(!configured.masking.mask_on_block);
    assert_eq!(configured.profile_name, None);
    assert_eq!(configured.violation_message_template, None);
    assert_eq!(configured.fallback_on_error, super::FallbackAction::Block);
    assert_eq!(configured.latest_role_message_only, None);
    assert!(!format!("{configured:?}").contains("secret-key"));
}

#[test]
fn panw_prisma_airs_requires_api_key_from_environment() {
    let loader = ConfigLoader {
        lookup: BTreeMap::new(),
    };
    let error = loader
        .load("guardrails:\n  - guardrail_name: prisma\n    litellm_params:\n      guardrail: panw_prisma_airs\n")
        .expect_err("missing key");
    assert!(
        matches!(&error, ConfigError::MissingEnvironment { variable, .. } if variable == "PANW_PRISMA_AIRS_API_KEY"),
        "{error:?}"
    );
}

#[test]
fn panw_prisma_airs_supports_environment_defaults_and_legacy_masking() {
    let loader = ConfigLoader {
        lookup: BTreeMap::from([
            (
                "PANW_PRISMA_AIRS_API_KEY".to_owned(),
                "env-secret".to_owned(),
            ),
            (
                "PANW_PRISMA_AIRS_API_BASE".to_owned(),
                "https://env.example.test".to_owned(),
            ),
        ]),
    };
    let config = loader.load("guardrails:\n  - guardrail_name: prisma\n    litellm_params:\n      guardrail: panw_prisma_airs\n      mask_on_block: true\n      app_name: gateway\n      fallback_on_error: allow\n      default_on: false\n").expect("config");
    let configured = config
        .sections
        .panw_prisma_airs
        .guardrails
        .first()
        .expect("guardrail");
    assert_eq!(configured.api_key.expose_secret(), "env-secret");
    assert_eq!(configured.api_base, "https://env.example.test");
    assert_eq!(configured.app_name, "LiteLLM-gateway");
    assert!(configured.masking.mask_on_block);
    assert!(configured.masking.content.request_content);
    assert!(configured.masking.content.response_content);
    assert_eq!(configured.fallback_on_error, super::FallbackAction::Allow);
    assert!(!configured.default_on);
}

#[test]
fn panw_prisma_airs_resolves_explicit_environment_references() {
    let loader = ConfigLoader {
        lookup: BTreeMap::from([
            ("CUSTOM_PANW_KEY".to_owned(), "private-key".to_owned()),
            (
                "CUSTOM_PANW_BASE".to_owned(),
                "https://custom.example.test".to_owned(),
            ),
        ]),
    };
    let config = loader
        .load("guardrails:\n  - guardrail_name: prisma\n    litellm_params:\n      guardrail: panw_prisma_airs\n      api_key: os.environ/CUSTOM_PANW_KEY\n      api_base: os.environ/CUSTOM_PANW_BASE\n")
        .expect("config");
    let configured = config
        .sections
        .panw_prisma_airs
        .guardrails
        .first()
        .expect("guardrail");
    assert_eq!(configured.api_key.expose_secret(), "private-key");
    assert_eq!(configured.api_base, "https://custom.example.test");
}

#[test]
fn panw_prisma_airs_explicit_values_override_defaults() {
    let loader = ConfigLoader {
        lookup: BTreeMap::new(),
    };
    let config = loader.load("guardrails:\n  - guardrail_name: custom\n    litellm_params:\n      guardrail: panw_prisma_airs\n      api_key: key\n      api_base: https://explicit.test\n      profile_name: profile\n      mask_request_content: true\n      mask_response_content: true\n      timeout: 0\n      violation_message_template: blocked\n      experimental_use_latest_role_message_only: true\n").expect("config");
    let configured = &config.sections.panw_prisma_airs.guardrails[0];
    assert_eq!(configured.guardrail_name, "custom");
    assert_eq!(configured.api_base, "https://explicit.test");
    assert_eq!(configured.profile_name.as_deref(), Some("profile"));
    assert!(configured.masking.content.request_content);
    assert!(configured.masking.content.response_content);
    assert_eq!(configured.timeout.to_bits(), 0.0_f64.to_bits());
    assert_eq!(
        configured.violation_message_template.as_deref(),
        Some("blocked")
    );
    assert_eq!(configured.fallback_on_error, super::FallbackAction::Block);
    assert_eq!(configured.latest_role_message_only, Some(true));
}

#[test]
fn panw_prisma_airs_timeout_resolves_from_environment() {
    let loader = ConfigLoader {
        lookup: BTreeMap::from([("PRISMA_TIMEOUT".to_owned(), "7.5".to_owned())]),
    };
    let config = loader.load("guardrails:\n  - guardrail_name: prisma\n    litellm_params:\n      guardrail: panw_prisma_airs\n      api_key: key\n      timeout: os.environ/PRISMA_TIMEOUT\n").expect("config");
    let configured = &config.sections.panw_prisma_airs.guardrails[0];
    assert_eq!(configured.timeout.to_bits(), 7.5_f64.to_bits());
}

#[test]
fn panw_prisma_airs_tolerates_other_guardrails_environment_timeout() {
    let loader = ConfigLoader {
        lookup: BTreeMap::from([("FIREWALL_TIMEOUT".to_owned(), "5".to_owned())]),
    };
    let config = loader.load("guardrails:\n  - guardrail_name: firewall\n    litellm_params:\n      guardrail: deepkeep\n      api_key: key\n      api_base: https://deepkeep.test\n      deepkeep_firewall_id: fixed\n      timeout: os.environ/FIREWALL_TIMEOUT\n").expect("config");
    assert!(config.sections.panw_prisma_airs.guardrails.is_empty());
}

#[rstest::rstest]
#[case::key("api_key", "[]")]
#[case::mask("mask_request_content", "bad")]
#[case::fallback("fallback_on_error", "ignore")]
#[case::timeout("timeout", "bad")]
fn panw_prisma_airs_rejects_wrong_field_types(#[case] field: &str, #[case] value: &str) {
    let loader = ConfigLoader {
        lookup: BTreeMap::new(),
    };
    let input = format!(
        "guardrails:\n  - litellm_params:\n      guardrail: panw_prisma_airs\n      {field}: {value}\n"
    );
    let error = loader.load(&input).expect_err("invalid field");
    assert!(error.to_string().contains(field), "{error}");
}
