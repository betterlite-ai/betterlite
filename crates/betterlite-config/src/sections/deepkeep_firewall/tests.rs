#![cfg(test)]

use std::collections::BTreeMap;
use std::time::Duration;

use crate::loader::{ConfigLoader, parse_view};
use rstest::rstest;
use secrecy::ExposeSecret as _;

#[test]
fn deepkeep_ui_friendly_name_matches_litellm() {
    assert_eq!(super::UI_FRIENDLY_NAME, "DeepKeep AI Firewall");
}

#[test]
fn deepkeep_config_normalizes_and_loads_required_values() {
    let source = "guardrails:\n  - guardrail_name: firewall\n    litellm_params:\n      guardrail: deepkeep\n      api_key: key\n      api_base: https://deepkeep.test///\n      deepkeep_firewall_id: configured\n      unreachable_fallback: fail_open\n      extra_headers:\n        X-Test: 'yes'\n";
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
fn deepkeep_raw_debug_never_shows_secrets() {
    let source = "guardrails:\n  - guardrail_name: firewall\n    litellm_params:\n      guardrail: deepkeep\n      api_key: top-secret-key\n      api_base: https://deepkeep.test\n      deepkeep_firewall_id: fixed\n      extra_headers:\n        Authorization: Bearer top-secret-token\n        X-API-Key: top-secret-header\n";
    let (raw, _ignored) = parse_view::<super::Raw>(source).expect("parses into Raw");
    let debugged = format!("{raw:?}");
    assert!(!debugged.contains("top-secret-key"));
    assert!(!debugged.contains("top-secret-token"));
    assert!(!debugged.contains("top-secret-header"));
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

#[rstest]
#[case::lower("fail_open")]
#[case::upper("FAIL_OPEN")]
#[case::mixed("Fail_Open")]
fn deepkeep_config_top_level_fallback(#[case] fallback: &str) {
    let source = format!(
        "guardrails:\n  - guardrail_name: firewall\n    litellm_params:\n      guardrail: deepkeep\n      api_key: key\n      api_base: https://deepkeep.test\n      deepkeep_firewall_id: fixed\n      unreachable_fallback: {fallback}\n"
    );
    let config = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load(&source)
    .expect("config");
    assert_eq!(
        config.sections.deepkeep_firewall.guardrails[0].unreachable_fallback,
        super::UnreachableFallback::FailOpen
    );
}

#[rstest]
#[case::integer("5", 5000)]
#[case::fraction("0.125", 125)]
#[case::numeric_string("'2.5'", 2500)]
#[case::null("null", 600000)]
#[case::empty("''", 600000)]
#[case::zero("0", 0)]
#[case::boolean("true", 1000)]
fn deepkeep_config_accepts_timeout_seconds(#[case] timeout: &str, #[case] milliseconds: u64) {
    let source = format!(
        "guardrails:\n  - guardrail_name: firewall\n    litellm_params:\n      guardrail: deepkeep\n      api_key: key\n      api_base: https://deepkeep.test\n      deepkeep_firewall_id: fixed\n      timeout: {timeout}\n"
    );
    let config = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load(&source)
    .expect("timeout config");
    assert_eq!(
        config.sections.deepkeep_firewall.guardrails[0].timeout,
        Duration::from_millis(milliseconds)
    );
}

#[test]
fn deepkeep_config_defaults_timeout_and_fallback() {
    let source = "guardrails:\n  - guardrail_name: firewall\n    litellm_params:\n      guardrail: deepkeep\n      api_key: key\n      api_base: https://deepkeep.test\n      deepkeep_firewall_id: fixed\n";
    let config = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load(source)
    .expect("config");
    let settings = &config.sections.deepkeep_firewall.guardrails[0];
    assert_eq!(settings.timeout, Duration::from_secs(600));
    assert_eq!(
        settings.unreachable_fallback,
        super::UnreachableFallback::FailClosed
    );
}

#[rstest]
#[case::negative_timeout("timeout: -1")]
#[case::nonfinite_timeout("timeout: .inf")]
#[case::nan_timeout("timeout: 'NaN'")]
#[case::overflow_timeout("timeout: 1e100")]
#[case::invalid_timeout("timeout: invalid")]
#[case::invalid_fallback("unreachable_fallback: invalid")]
#[case::nested_fallback("optional_params: {unreachable_fallback: fail_open}")]
fn deepkeep_config_rejects_invalid_parameters(#[case] parameter: &str) {
    let source = format!(
        "guardrails:\n  - guardrail_name: firewall\n    litellm_params:\n      guardrail: deepkeep\n      api_key: key\n      api_base: https://deepkeep.test\n      deepkeep_firewall_id: fixed\n      {parameter}\n"
    );
    assert!(
        ConfigLoader {
            lookup: BTreeMap::new()
        }
        .load(&source)
        .is_err()
    );
}
