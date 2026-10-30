#![cfg(test)]

use std::collections::BTreeMap;

use crate::loader::ConfigLoader;
use secrecy::ExposeSecret as _;

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
    assert_eq!(settings.extra_headers["X-Test"], "yes");
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
