#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::sync::Arc;
use std::time::Duration;

use crate::loader::{ConfigLoader, parse_view};
use rstest::rstest;
use secrecy::ExposeSecret as _;
use tempfile::NamedTempFile;
use tracing::subscriber::with_default;

fn logged<T, F>(action: F) -> (T, String)
where
    F: FnOnce() -> T,
{
    let log = NamedTempFile::new().unwrap();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_writer(Arc::new(log.reopen().unwrap()))
        .finish();
    let result = with_default(subscriber, action);
    (result, fs::read_to_string(log.path()).unwrap())
}

#[test]
fn deepkeep_ui_friendly_name_matches_litellm() {
    assert_eq!(super::UI_FRIENDLY_NAME, "DeepKeep AI Firewall");
}

#[test]
fn deepkeep_config_normalizes_and_loads_required_values() {
    let source = "guardrails:\n  - guardrail_name: firewall\n    litellm_params:\n      guardrail: deepkeep\n      api_key: key\n      api_base: https://deepkeep.test///\n      deepkeep_firewall_id: configured\n      unreachable_fallback: fail_open\n";
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
fn deepkeep_config_warns_and_ignores_extra_headers_as_list() {
    let source = "guardrails:\n  - guardrail_name: firewall\n    litellm_params:\n      guardrail: deepkeep\n      api_key: key\n      api_base: https://deepkeep.test\n      deepkeep_firewall_id: configured\n      extra_headers:\n        - X-Test\n";
    let (config, log) = logged(|| {
        ConfigLoader {
            lookup: BTreeMap::new(),
        }
        .load(source)
    });
    config.expect("config");
    assert!(log.contains("DeepKeep extra_headers list ignored"));
}

fn reject(parameter: &str) -> String {
    let source = format!(
        "guardrails:\n  - guardrail_name: firewall\n    litellm_params:\n      guardrail: deepkeep\n      api_key: key\n      api_base: https://deepkeep.test\n      deepkeep_firewall_id: fixed\n      {parameter}\n"
    );
    ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load(&source)
    .expect_err("invalid configuration")
    .to_string()
}

#[rstest]
#[case::mapping("extra_headers:\n        Authorization: Bearer token")]
#[case::string("extra_headers: not-a-list")]
fn deepkeep_config_rejects_extra_headers_mapping(#[case] parameter: &str) {
    assert!(reject(parameter).contains("extra_headers"));
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
    let source = "guardrails:\n  - guardrail_name: firewall\n    litellm_params:\n      guardrail: deepkeep\n      api_key: top-secret-key\n      api_base: https://deepkeep.test\n      deepkeep_firewall_id: fixed\n";
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
}

#[test]
fn deepkeep_raw_debug_never_shows_secrets() {
    let source = "guardrails:\n  - guardrail_name: firewall\n    litellm_params:\n      guardrail: deepkeep\n      api_key: top-secret-key\n      api_base: https://deepkeep.test\n      deepkeep_firewall_id: fixed\n";
    let (raw, _ignored) = parse_view::<super::Raw>(source).expect("parses into Raw");
    let debugged = format!("{raw:?}");
    assert!(!debugged.contains("top-secret-key"));
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

#[test]
fn deepkeep_config_reads_timeout_from_environment() {
    let source = "guardrails:\n  - guardrail_name: firewall\n    litellm_params:\n      guardrail: deepkeep\n      api_key: key\n      api_base: https://deepkeep.test\n      deepkeep_firewall_id: fixed\n      timeout: os.environ/FIREWALL_TIMEOUT\n";
    let config = ConfigLoader {
        lookup: BTreeMap::from([("FIREWALL_TIMEOUT".to_owned(), "5".to_owned())]),
    }
    .load(source)
    .expect("config");
    assert_eq!(
        config.sections.deepkeep_firewall.guardrails[0].timeout,
        Duration::from_secs(5)
    );
}

#[test]
fn deepkeep_config_reads_fallback_from_environment() {
    let source = "guardrails:\n  - guardrail_name: firewall\n    litellm_params:\n      guardrail: deepkeep\n      api_key: key\n      api_base: https://deepkeep.test\n      deepkeep_firewall_id: fixed\n      unreachable_fallback: os.environ/FIREWALL_FALLBACK\n";
    let config = ConfigLoader {
        lookup: BTreeMap::from([("FIREWALL_FALLBACK".to_owned(), "fail_open".to_owned())]),
    }
    .load(source)
    .expect("config");
    assert_eq!(
        config.sections.deepkeep_firewall.guardrails[0].unreachable_fallback,
        super::UnreachableFallback::FailOpen
    );
}

#[rstest]
#[case::timeout("timeout: os.environ/FIREWALL_TIMEOUT")]
#[case::fallback("unreachable_fallback: os.environ/FIREWALL_FALLBACK")]
fn deepkeep_config_rejects_unset_environment_reference(#[case] parameter: &str) {
    let variable = parameter.split('/').nth(1).expect("variable name");
    assert!(reject(parameter).contains(variable));
}

#[rstest]
#[case::negative_timeout("timeout: -1")]
#[case::timeout_reference("timeout: os.environ/OTHER_TIMEOUT")]
#[case::fallback_reference("unreachable_fallback: os.environ/OTHER_FALLBACK")]
fn deepkeep_config_leaves_other_guardrails_values_alone(#[case] parameter: &str) {
    let source = format!(
        "guardrails:\n  - guardrail_name: safety\n    litellm_params:\n      guardrail: lasso\n      api_key: key\n      {parameter}\n"
    );
    let config = ConfigLoader {
        lookup: BTreeMap::from([
            ("OTHER_TIMEOUT".to_owned(), "not-a-number".to_owned()),
            ("OTHER_FALLBACK".to_owned(), "not-fail-open".to_owned()),
        ]),
    }
    .load(&source)
    .expect("config");
    assert!(config.sections.deepkeep_firewall.guardrails.is_empty());
}

#[test]
fn deepkeep_config_ignores_nested_optional_params_fallback() {
    let source = "guardrails:\n  - guardrail_name: firewall\n    litellm_params:\n      guardrail: deepkeep\n      api_key: key\n      api_base: https://deepkeep.test\n      deepkeep_firewall_id: fixed\n      optional_params:\n        unreachable_fallback: fail_open\n";
    let (loaded, log) = logged(|| {
        ConfigLoader {
            lookup: BTreeMap::new(),
        }
        .load(source)
    });
    let config = loaded.expect("config");
    let settings = &config.sections.deepkeep_firewall.guardrails[0];
    assert_eq!(
        settings.unreachable_fallback,
        super::UnreachableFallback::FailClosed
    );
    assert!(log.contains("optional_params"));
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
#[case::null("null", 600_000)]
#[case::empty("''", 600_000)]
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
#[case::no_trailing_slash("https://deepkeep.test/v3/openai/beta/litellm_basic_guardrail_api")]
#[case::trailing_slash("https://deepkeep.test/v3/openai/beta/litellm_basic_guardrail_api/")]
fn deepkeep_config_keeps_api_base_that_ends_with_endpoint(#[case] api_base: &str) {
    let source = format!(
        "guardrails:\n  - guardrail_name: firewall\n    litellm_params:\n      guardrail: deepkeep\n      api_key: key\n      api_base: {api_base}\n      deepkeep_firewall_id: fixed\n"
    );
    let config = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load(&source)
    .expect("config");
    assert_eq!(
        config.sections.deepkeep_firewall.guardrails[0].api_base,
        "https://deepkeep.test/v3/openai/beta/litellm_basic_guardrail_api"
    );
}

#[rstest]
#[case::negative_timeout("timeout: -1")]
#[case::nonfinite_timeout("timeout: .inf")]
#[case::nan_timeout("timeout: 'NaN'")]
#[case::overflow_timeout("timeout: 1e100")]
#[case::invalid_timeout("timeout: invalid")]
#[case::invalid_fallback("unreachable_fallback: invalid")]
fn deepkeep_config_rejects_invalid_parameters(#[case] parameter: &str) {
    let source = format!(
        "guardrails:\n  - guardrail_name: firewall\n    litellm_params:\n      guardrail: deepkeep\n      api_key: key\n      api_base: https://deepkeep.test\n      deepkeep_firewall_id: fixed\n      {parameter}\n"
    );
    let error = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load(&source)
    .expect_err("invalid parameter");
    let (field, _value) = parameter.split_once(':').expect("parameter name");
    assert!(error.to_string().contains(field));
}
