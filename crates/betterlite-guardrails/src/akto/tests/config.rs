use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_config::loader::{ConfigError, ConfigLoader};
use betterlite_config::sections::akto_mirroring::{Configured, UnreachableFallback};
use rstest::rstest;
use secrecy::ExposeSecret as _;

fn config_source(extra: &str) -> String {
    format!(
        "guardrails:\n  - guardrail_name: akto-validate\n    litellm_params:\n      guardrail: akto\n{extra}"
    )
}

fn load_entry(source: &str, lookup: BTreeMap<String, String>) -> Configured {
    let config = ConfigLoader { lookup }.load(source).expect("config");
    config.sections.akto_mirroring.guardrails[0].clone()
}

#[test]
fn akto_guardrail_config_loads_the_required_keys_and_defaults() {
    let source = config_source(
        "      akto_base_url: https://akto.test///\n      akto_api_key: secret-key\n",
    );
    let configured = load_entry(&source, BTreeMap::new());
    assert_eq!(configured.guardrail_name, "akto-validate");
    assert_eq!(configured.api_base, "https://akto.test");
    assert_eq!(configured.api_key.expose_secret(), "secret-key");
    assert_eq!(configured.account_id, "1000000");
    assert_eq!(configured.vxlan_id, "0");
    assert_eq!(
        configured.unreachable_fallback,
        UnreachableFallback::FailClosed
    );
    assert_eq!(configured.timeout, Duration::from_secs(5));
    assert!(!format!("{configured:?}").contains("secret-key"));
}

#[test]
fn akto_guardrail_config_reads_the_environment() {
    let lookup = BTreeMap::from([
        (
            "AKTO_GUARDRAIL_API_BASE".to_owned(),
            "http://env.test".to_owned(),
        ),
        ("AKTO_API_KEY".to_owned(), "env-key".to_owned()),
        ("AKTO_ACCOUNT_ID".to_owned(), "4242".to_owned()),
        ("AKTO_VXLAN_ID".to_owned(), "9".to_owned()),
    ]);
    let configured = load_entry(&config_source(""), lookup);
    assert_eq!(configured.api_base, "http://env.test");
    assert_eq!(configured.api_key.expose_secret(), "env-key");
    assert_eq!(configured.account_id, "4242");
    assert_eq!(configured.vxlan_id, "9");
}

#[test]
fn akto_guardrail_config_keeps_configured_ids() {
    let source = config_source(
        "      akto_base_url: http://akto.test\n      akto_api_key: key\n      akto_account_id: '4242'\n      akto_vxlan_id: '7'\n",
    );
    let configured = load_entry(&source, BTreeMap::new());
    assert_eq!(configured.account_id, "4242");
    assert_eq!(configured.vxlan_id, "7");
}

#[test]
fn akto_guardrail_config_requires_a_base_url() {
    let source = config_source("      akto_api_key: key\n");
    let error = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load(&source)
    .expect_err("base url");
    assert!(
        matches!(&error, ConfigError::MissingEnvironment { variable, .. } if variable == "AKTO_GUARDRAIL_API_BASE"),
        "{error:?}"
    );
}

#[test]
fn akto_guardrail_config_requires_an_api_key() {
    let source = config_source("      akto_base_url: http://akto.test\n");
    let error = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load(&source)
    .expect_err("api key");
    assert!(
        matches!(&error, ConfigError::MissingEnvironment { variable, .. } if variable == "AKTO_API_KEY"),
        "{error:?}"
    );
}

#[rstest]
#[case::fail_open("fail_open", UnreachableFallback::FailOpen)]
#[case::fail_closed("fail_closed", UnreachableFallback::FailClosed)]
#[case::environment("os.environ/AKTO_FALLBACK", UnreachableFallback::FailOpen)]
fn akto_guardrail_config_reads_the_fallback(
    #[case] value: &str,
    #[case] expected: UnreachableFallback,
) {
    let source = config_source(&format!(
        "      akto_base_url: http://akto.test\n      akto_api_key: key\n      unreachable_fallback: {value}\n"
    ));
    let lookup = BTreeMap::from([("AKTO_FALLBACK".to_owned(), "fail_open".to_owned())]);
    assert_eq!(load_entry(&source, lookup).unreachable_fallback, expected);
}

#[test]
fn akto_guardrail_config_rejects_an_unknown_fallback() {
    let source = config_source(
        "      akto_base_url: http://akto.test\n      akto_api_key: key\n      unreachable_fallback: sometimes\n",
    );
    let error = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load(&source)
    .expect_err("fallback");
    assert!(matches!(error, ConfigError::Yaml { .. }));
}

#[test]
fn akto_guardrail_config_reads_an_integer_timeout() {
    let source = config_source(
        "      akto_base_url: http://akto.test\n      akto_api_key: key\n      guardrail_timeout: 7\n",
    );
    assert_eq!(
        load_entry(&source, BTreeMap::new()).timeout,
        Duration::from_secs(7)
    );
}

#[test]
fn akto_guardrail_config_reads_a_timeout_from_the_environment() {
    let lookup = BTreeMap::from([("AKTO_TIMEOUT".to_owned(), "11".to_owned())]);
    let source = config_source(
        "      akto_base_url: http://akto.test\n      akto_api_key: key\n      guardrail_timeout: os.environ/AKTO_TIMEOUT\n",
    );
    assert_eq!(load_entry(&source, lookup).timeout, Duration::from_secs(11));
}

#[test]
fn akto_guardrail_config_rejects_a_negative_timeout() {
    let source = config_source(
        "      akto_base_url: http://akto.test\n      akto_api_key: key\n      guardrail_timeout: -2\n",
    );
    let error = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load(&source)
    .expect_err("timeout");
    assert!(matches!(error, ConfigError::Yaml { .. }));
}

#[test]
fn akto_guardrail_config_leaves_other_guardrails_to_their_sections() {
    let source = "guardrails:\n  - guardrail_name: firewall\n    litellm_params:\n      guardrail: deepkeep\n      akto_base_url: http://other.test\n  - guardrail_name: akto\n    litellm_params:\n      guardrail: akto\n      akto_base_url: http://akto.test\n      akto_api_key: key\n";
    let config = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load(source)
    .expect("config");
    let guardrails = config.sections.akto_mirroring.guardrails;
    assert_eq!(guardrails.len(), 1);
    assert_eq!(guardrails[0].guardrail_name, "akto");
}

#[test]
fn akto_guardrail_config_resolves_an_environment_reference_in_a_value() {
    let lookup = BTreeMap::from([("AKTO_BASE".to_owned(), "http://resolved.test".to_owned())]);
    let source =
        config_source("      akto_base_url: os.environ/AKTO_BASE\n      akto_api_key: key\n");
    let configured = load_entry(&source, lookup);
    assert_eq!(configured.api_base, "http://resolved.test");
}
