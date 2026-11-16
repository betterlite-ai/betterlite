#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;
use secrecy::ExposeSecret as _;

use super::{Settings, UnreachableFallback};
use crate::loader::{ConfigError, ConfigLoader, parse_view};
use crate::sections::Section as _;
use crate::sections::guardrail_identity::item::GuardrailValue;

const HEAD: &str = "guardrails:\n  - guardrail_name: squeeze\n    litellm_params:\n      guardrail: compresr\n      mode: pre_call\n";

fn lookup(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn resolve(params: &str, environment: &[(&str, &str)]) -> Result<Settings, ConfigError> {
    let source = format!("{HEAD}{params}");
    let (raw, _ignored) = parse_view::<super::Raw>(&source)?;
    raw.convert(&lookup(environment))
}

fn load(params: &str) -> Result<(), ConfigError> {
    let source = format!("{HEAD}{params}");
    ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load(&source)
    .map(drop)
}

#[test]
fn compresr_config_skips_other_guardrails_and_absent_sections() {
    let other = "guardrails:\n  - guardrail_name: aim\n    litellm_params:\n      guardrail: aim\n";
    let (raw, _ignored) = parse_view::<super::Raw>(other).unwrap();
    assert!(raw.convert(&lookup(&[])).unwrap().guardrails.is_empty());
    let (absent, _ignored) =
        parse_view::<super::Raw>("general_settings:\n  master_key: key\n").unwrap();
    assert!(absent.convert(&lookup(&[])).unwrap().guardrails.is_empty());
}

#[test]
fn compresr_config_without_tuning_leaves_every_knob_unset() {
    let settings = resolve("", &[]).unwrap();
    let configured = settings.guardrails.first().unwrap();
    assert_eq!(configured.guardrail_name, "squeeze");
    assert!(configured.api_base.is_none() && configured.api_key.is_none());
    assert!(configured.model.is_none() && configured.timeout.is_none());
    assert_eq!(
        configured.unreachable_fallback,
        UnreachableFallback::FailClosed
    );
    let tuning = &configured.tuning;
    assert!(tuning.target_compression_ratio.is_none() && tuning.min_chars_to_compress.is_none());
    assert!(
        tuning.coarse.is_none() && tuning.dynamic.is_none() && tuning.enable_retrieval.is_none()
    );
    assert!(tuning.max_bytes_per_call.is_none() && tuning.compression_params.is_none());
}

#[test]
fn compresr_config_reads_every_flat_key() {
    let params = "      api_base: https://compresr.test\n      api_key: secret-key\n      model: latte_v1\n      unreachable_fallback: fail_open\n      timeout: 2.5\n      target_compression_ratio: 4\n      coarse: false\n      min_chars_to_compress: 100\n      compress_tool_outputs: false\n      compress_system: true\n      compress_history: true\n      compress_last_user: true\n      enable_retrieval: false\n      max_bytes_per_call: 2048\n      allow_bypass_header: true\n      dynamic: false\n      dynamic_min_ratio: 1.5\n      dynamic_max_ratio: 8\n      compression_params:\n        heuristic_chunking: true\n";
    let settings = resolve(params, &[]).unwrap();
    let configured = settings.guardrails.first().unwrap();
    assert_eq!(
        configured.api_base.as_deref(),
        Some("https://compresr.test")
    );
    assert_eq!(
        configured
            .api_key
            .as_ref()
            .map(|key| key.expose_secret().to_owned()),
        Some("secret-key".to_owned())
    );
    assert_eq!(configured.model.as_deref(), Some("latte_v1"));
    assert_eq!(
        configured.unreachable_fallback,
        UnreachableFallback::FailOpen
    );
    assert_eq!(configured.timeout, Some(2.5_f64));
    let tuning = &configured.tuning;
    assert_eq!(tuning.target_compression_ratio, Some(4.0_f64));
    assert_eq!(tuning.coarse, Some(false));
    assert_eq!(tuning.min_chars_to_compress, Some(100));
    assert_eq!(tuning.compress_tool_outputs, Some(false));
    assert_eq!(tuning.compress_system, Some(true));
    assert_eq!(tuning.compress_history, Some(true));
    assert_eq!(tuning.compress_last_user, Some(true));
    assert_eq!(tuning.enable_retrieval, Some(false));
    assert_eq!(tuning.max_bytes_per_call, Some(2048));
    assert_eq!(tuning.allow_bypass_header, Some(true));
    assert_eq!(tuning.dynamic, Some(false));
    assert_eq!(tuning.dynamic_min_ratio, Some(1.5_f64));
    assert_eq!(tuning.dynamic_max_ratio, Some(8.0_f64));
    let passthrough = tuning.compression_params.as_ref().unwrap();
    assert_eq!(
        passthrough.get("heuristic_chunking"),
        Some(&GuardrailValue::Boolean(true))
    );
}

#[test]
fn compresr_config_prefers_the_nested_optional_params() {
    let params = "      coarse: true\n      min_chars_to_compress: 100\n      dynamic: true\n      optional_params:\n        coarse: false\n        max_bytes_per_call: 64\n        compression_params:\n          nested: 1\n      compression_params:\n        flat: 1\n";
    let settings = resolve(params, &[]).unwrap();
    let tuning = &settings.guardrails.first().unwrap().tuning;
    assert_eq!(tuning.coarse, Some(false));
    assert_eq!(tuning.min_chars_to_compress, Some(100));
    assert_eq!(tuning.dynamic, Some(true));
    assert_eq!(tuning.max_bytes_per_call, Some(64));
    let passthrough = tuning.compression_params.as_ref().unwrap();
    assert!(passthrough.contains_key("nested") && !passthrough.contains_key("flat"));
}

#[test]
fn compresr_config_resolves_environment_references() {
    let params = "      api_base: os.environ/BASE\n      api_key: os.environ/KEY\n      model: os.environ/MODEL\n      unreachable_fallback: os.environ/FALLBACK\n      timeout: os.environ/TIMEOUT\n      target_compression_ratio: os.environ/RATIO\n      min_chars_to_compress: os.environ/MIN\n      coarse: os.environ/COARSE\n      optional_params:\n        enable_retrieval: os.environ/RETRIEVAL\n        max_bytes_per_call: os.environ/CAP\n";
    let environment = [
        ("BASE", "https://env.test"),
        ("KEY", "env-key"),
        ("MODEL", "env-model"),
        ("FALLBACK", "fail_open"),
        ("TIMEOUT", "7.5"),
        ("RATIO", "0.25"),
        ("MIN", "12"),
        ("COARSE", "no"),
        ("RETRIEVAL", "off"),
        ("CAP", "4096"),
    ];
    let settings = resolve(params, &environment).unwrap();
    let configured = settings.guardrails.first().unwrap();
    assert_eq!(configured.api_base.as_deref(), Some("https://env.test"));
    assert_eq!(
        configured
            .api_key
            .as_ref()
            .map(|key| key.expose_secret().to_owned()),
        Some("env-key".to_owned())
    );
    assert_eq!(configured.model.as_deref(), Some("env-model"));
    assert_eq!(
        configured.unreachable_fallback,
        UnreachableFallback::FailOpen
    );
    assert_eq!(configured.timeout, Some(7.5_f64));
    let tuning = &configured.tuning;
    assert_eq!(tuning.target_compression_ratio, Some(0.25_f64));
    assert_eq!(tuning.min_chars_to_compress, Some(12));
    assert_eq!(tuning.coarse, Some(false));
    assert_eq!(tuning.enable_retrieval, Some(false));
    assert_eq!(tuning.max_bytes_per_call, Some(4096));
}

#[test]
fn compresr_config_empty_references_count_as_unset() {
    let params = "      api_base: os.environ/BASE\n      model: os.environ/MODEL\n      unreachable_fallback: os.environ/FALLBACK\n      coarse: os.environ/COARSE\n";
    let environment = [
        ("BASE", ""),
        ("MODEL", ""),
        ("FALLBACK", ""),
        ("COARSE", ""),
    ];
    let settings = resolve(params, &environment).unwrap();
    let configured = settings.guardrails.first().unwrap();
    assert!(configured.api_base.is_none() && configured.model.is_none());
    assert_eq!(
        configured.unreachable_fallback,
        UnreachableFallback::FailClosed
    );
    assert!(configured.tuning.coarse.is_none());
}

#[rstest]
#[case::fallback("      unreachable_fallback: never\n", "unreachable_fallback")]
#[case::ratio("      target_compression_ratio: lots\n", "target_compression_ratio")]
#[case::minimum("      min_chars_to_compress: 1.5\n", "min_chars_to_compress")]
#[case::cap(
    "      optional_params:\n        max_bytes_per_call: huge\n",
    "max_bytes_per_call"
)]
#[case::flag("      optional_params:\n        coarse: maybe\n", "coarse")]
#[case::missing_variable("      api_key: os.environ/ABSENT\n", "ABSENT")]
fn compresr_config_rejects_a_bad_value(#[case] params: &str, #[case] named: &str) {
    let error = resolve(params, &[]).unwrap_err();
    assert!(error.to_string().contains(named), "{error}");
}

#[test]
fn no_compresr_key_is_unsupported() {
    load("      api_base: https://compresr.test\n      api_key: secret\n      model: latte_v2\n      timeout: 5\n      unreachable_fallback: fail_open\n      target_compression_ratio: 0.5\n      coarse: true\n      min_chars_to_compress: 500\n      compress_tool_outputs: true\n      compress_system: false\n      compress_history: false\n      compress_last_user: false\n      enable_retrieval: true\n      max_bytes_per_call: 1024\n      allow_bypass_header: false\n      dynamic: true\n      dynamic_min_ratio: 1.5\n      dynamic_max_ratio: 10\n      compression_params:\n        anything: goes\n      optional_params:\n        coarse: false\n        compression_params:\n          anything: else\n").unwrap();
}

#[rstest]
#[case::flat("      bogus_key: 1\n", "guardrails.0.litellm_params.bogus_key")]
#[case::nested(
    "      optional_params:\n        bogus_key: 1\n",
    "guardrails.0.litellm_params.optional_params.bogus_key"
)]
fn a_misspelled_compresr_key_is_unsupported(#[case] params: &str, #[case] path: &str) {
    let ConfigError::Unsupported { paths } = load(params).unwrap_err() else {
        panic!("the misspelled key must be reported as unsupported");
    };
    assert_eq!(paths, [path]);
}
