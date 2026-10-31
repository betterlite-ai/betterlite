#![cfg(test)]

use std::collections::BTreeMap;

use crate::loader::ConfigLoader;
use rstest::rstest;
use secrecy::ExposeSecret as _;

#[test]
fn deepkeep_ui_friendly_name_matches_litellm() {
    assert_eq!(super::UI_FRIENDLY_NAME, "DeepKeep AI Firewall");
}

#[test]
fn deepkeep_config_normalizes_and_loads_required_values() {
    let source = "guardrails:\n  - guardrail_name: firewall\n    litellm_params:\n      guardrail: deepkeep\n      api_key: key\n      api_base: https://deepkeep.test///\n      deepkeep_firewall_id: configured\n      optional_params:\n        unreachable_fallback: fail_open\n      extra_headers:\n        X-Test: 'yes'\n";
    let config = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load(source)
    .expect("config");
    let settings = config
        .sections
        .deepkeep_firewall
        .guardrails
        .first()
        .expect("entry");
    assert_eq!(settings.api_key.expose_secret(), "key");
    assert_eq!(
        settings.api_base,
        "https://deepkeep.test/v3/openai/beta/litellm_basic_guardrail_api"
    );
    assert_eq!(settings.extra_headers["X-Test"].expose_secret(), "yes");
}

#[test]
fn deepkeep_config_requires_each_secret() {
    let loader = ConfigLoader {
        lookup: BTreeMap::new(),
    };
    let error = loader
        .load(
            "guardrails:\n  - guardrail_name: firewall\n    litellm_params:\n      guardrail: deepkeep\n",
        )
        .expect_err("missing configuration");
    assert!(error.to_string().contains("DEEPKEEP_API_KEY"));
}

#[test]
fn deepkeep_config_rejects_extra_headers_as_list() {
    let source = "guardrails:\n  - guardrail_name: firewall\n    litellm_params:\n      guardrail: deepkeep\n      api_key: key\n      api_base: https://deepkeep.test\n      deepkeep_firewall_id: configured\n      extra_headers:\n        - X-Test\n";
    let config = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load(source)
    .expect("config");
    let settings = config
        .sections
        .deepkeep_firewall
        .guardrails
        .first()
        .expect("entry");
    assert!(settings.extra_headers.is_empty());
}

#[rstest]
#[case::api_key("DEEPKEEP_API_KEY")]
#[case::api_base("DEEPKEEP_API_BASE")]
#[case::firewall_id("DEEPKEEP_FIREWALL_ID")]
fn deepkeep_config_rejects_empty_environment_value(#[case] variable: &str) {
    let mut lookup = BTreeMap::from([
        ("DEEPKEEP_API_KEY".to_owned(), "key".to_owned()),
        (
            "DEEPKEEP_API_BASE".to_owned(),
            "https://deepkeep.test".to_owned(),
        ),
        ("DEEPKEEP_FIREWALL_ID".to_owned(), "firewall".to_owned()),
    ]);
    lookup.insert(variable.to_owned(), String::new());
    let error = ConfigLoader { lookup }
        .load("guardrails:\n  - guardrail_name: firewall\n    litellm_params:\n      guardrail: deepkeep\n")
        .expect_err("empty environment value is not configuration");
    assert!(error.to_string().contains(variable));
}

#[test]
fn deepkeep_config_debug_never_shows_secrets() {
    let source = "guardrails:\n  - guardrail_name: firewall\n    litellm_params:\n      guardrail: deepkeep\n      api_key: top-secret-key\n      api_base: https://deepkeep.test\n      deepkeep_firewall_id: fixed\n      extra_headers:\n        Authorization: Bearer top-secret-token\n";
    let config = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load(source)
    .expect("config");
    let settings = config
        .sections
        .deepkeep_firewall
        .guardrails
        .first()
        .expect("entry");
    let debugged = format!("{settings:?}");
    assert!(!debugged.contains("top-secret-key"));
    assert!(!debugged.contains("top-secret-token"));
}

#[test]
fn deepkeep_config_reads_environment() {
    let config = ConfigLoader {
        lookup: BTreeMap::from([
            ("DEEPKEEP_API_KEY".to_owned(), "key".to_owned()),
            ("DEEPKEEP_API_BASE".to_owned(), "https://deepkeep.test".to_owned()),
            ("DEEPKEEP_FIREWALL_ID".to_owned(), "firewall".to_owned()),
        ]),
    }
    .load("guardrails:\n  - guardrail_name: firewall\n    litellm_params:\n      guardrail: deepkeep\n")
    .expect("config");
    assert_eq!(
        config.sections.deepkeep_firewall.guardrails[0].deepkeep_firewall_id,
        "firewall"
    );
}
