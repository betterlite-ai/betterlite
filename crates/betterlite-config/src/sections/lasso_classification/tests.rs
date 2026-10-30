#![cfg(test)]

use std::collections::BTreeMap;

use crate::loader::{ConfigError, ConfigLoader};
use secrecy::ExposeSecret as _;

const LASSO_CONFIG: &str =
    "guardrails:\n  - guardrail_name: safety\n    litellm_params:\n      guardrail: lasso\n";

#[test]
fn lasso_config_names_the_guardrail_for_the_ui() {
    assert_eq!(super::UI_FRIENDLY_NAME, "Lasso Guardrail");
}

#[test]
fn lasso_config_rejects_other_guardrail_kinds() {
    let loader = ConfigLoader {
        lookup: BTreeMap::new(),
    };
    let source = LASSO_CONFIG.replace("guardrail: lasso", "guardrail: aim");
    let error = loader.load(&source).unwrap_err();
    assert!(matches!(error, ConfigError::Unsupported { paths }
        if paths == ["guardrails.0.litellm_params.guardrail"]));
}

#[test]
fn lasso_config_is_optional() {
    let loader = ConfigLoader {
        lookup: BTreeMap::new(),
    };
    loader
        .load("general_settings:\n  master_key: test-master\n")
        .unwrap();
}

#[test]
fn lasso_config_loads_environment() {
    let loader = ConfigLoader {
        lookup: BTreeMap::from([
            ("LASSO_API_KEY".to_owned(), "key".to_owned()),
            (
                "LASSO_API_BASE".to_owned(),
                "https://example.test".to_owned(),
            ),
            ("LASSO_USER_ID".to_owned(), "user".to_owned()),
            ("LASSO_CONVERSATION_ID".to_owned(), "session".to_owned()),
        ]),
    };
    let config = loader.load(LASSO_CONFIG).unwrap();
    let settings = config
        .sections
        .lasso_classification
        .guardrails
        .first()
        .unwrap();
    assert_eq!(settings.guardrail_name, "safety");
    assert_eq!(settings.api_key.expose_secret(), "key");
    assert_eq!(settings.api_base, "https://example.test");
    assert_eq!(settings.user_id.as_deref(), Some("user"));
    assert_eq!(settings.conversation_id.as_deref(), Some("session"));
    assert!(!settings.mask);
}

#[test]
fn lasso_config_requires_key_only_when_enabled() {
    let loader = ConfigLoader {
        lookup: BTreeMap::new(),
    };
    let error = loader.load(LASSO_CONFIG).unwrap_err();
    assert!(
        matches!(error, ConfigError::MissingEnvironment { variable, path }
        if variable == "LASSO_API_KEY" && path == "guardrails.0.litellm_params.api_key")
    );
}

#[test]
fn lasso_config_retains_unknown_paths() {
    let loader = ConfigLoader {
        lookup: BTreeMap::new(),
    };
    let source = format!("{LASSO_CONFIG}      api_key: key\n      misspelled: true\n");
    let error = loader.load(&source).unwrap_err();
    assert!(matches!(error, ConfigError::Unsupported { paths }
        if paths == ["guardrails.0.litellm_params.misspelled"]));
}

#[test]
fn lasso_config_leaves_deepkeep_entries_to_deepkeep_section() {
    let loader = ConfigLoader {
        lookup: BTreeMap::new(),
    };
    let source = "guardrails:\n  - guardrail_name: safety\n    litellm_params:\n      guardrail: lasso\n      api_key: key\n  - guardrail_name: firewall\n    litellm_params:\n      guardrail: deepkeep\n      api_key: key\n      api_base: https://deepkeep.test\n      deepkeep_firewall_id: fixed\n";
    let config = loader.load(source).unwrap();
    let lasso = config
        .sections
        .lasso_classification
        .guardrails
        .first()
        .unwrap();
    assert_eq!(lasso.guardrail_name, "safety");
    assert_eq!(config.sections.lasso_classification.guardrails.len(), 1);
    assert_eq!(config.sections.deepkeep_firewall.guardrails.len(), 1);
}

#[test]
fn lasso_config_explicit_values_win() {
    let loader = ConfigLoader {
        lookup: BTreeMap::from([("LASSO_API_KEY".to_owned(), "environment".to_owned())]),
    };
    let source = format!(
        "{LASSO_CONFIG}      api_key: configured\n      lasso_user_id: user\n      lasso_conversation_id: session\n      mask: true\n"
    );
    let config = loader.load(&source).unwrap();
    let settings = config
        .sections
        .lasso_classification
        .guardrails
        .first()
        .unwrap();
    assert_eq!(settings.api_key.expose_secret(), "configured");
    assert_eq!(
        settings.api_base,
        "https://server.lasso.security/gateway/v3"
    );
    assert_eq!(settings.user_id.as_deref(), Some("user"));
    assert_eq!(settings.conversation_id.as_deref(), Some("session"));
    assert!(settings.mask);
}
