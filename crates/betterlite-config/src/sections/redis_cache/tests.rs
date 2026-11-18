#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use crate::loader::{ConfigError, ConfigLoader};

fn settings(source: &str, environment: BTreeMap<String, String>) -> super::Settings {
    ConfigLoader {
        lookup: environment,
    }
    .load(source)
    .expect("config should load")
    .sections
    .redis_cache
}

fn loaded(source: &str) -> super::Settings {
    settings(source, BTreeMap::new())
}

fn failure(source: &str, environment: BTreeMap<String, String>) -> ConfigError {
    ConfigLoader {
        lookup: environment,
    }
    .load(source)
    .expect_err("config should fail")
}

#[test]
fn ttl_defaults_to_sixty() {
    assert_eq!(loaded("{}").default_ttl, 60);
}

#[rstest]
#[case::integer("90", 90)]
#[case::float("90.0", 90)]
#[case::exponent("9e1", 90)]
fn ttl_reads_the_litellm_settings_key(#[case] value: &str, #[case] expected: i64) {
    let source = format!("litellm_settings:\n  default_redis_ttl: {value}\n");
    assert_eq!(loaded(&source).default_ttl, expected);
}

#[rstest]
#[case("60.9", 60)]
#[case("0.9", 0)]
#[case("59.999", 59)]
fn ttl_cuts_a_float_toward_zero(#[case] value: &str, #[case] expected: i64) {
    let source = format!("litellm_settings:\n  default_redis_ttl: {value}\n");
    assert_eq!(loaded(&source).default_ttl, expected);
}

#[rstest]
#[case("-1.9", -1)]
#[case("-0.5", 0)]
#[case("-60.9", -60)]
fn ttl_cuts_a_negative_float_up(#[case] value: &str, #[case] expected: i64) {
    let source = format!("litellm_settings:\n  default_redis_ttl: {value}\n");
    assert_eq!(loaded(&source).default_ttl, expected);
}

#[rstest]
#[case::integer("0")]
#[case::float("0.0")]
#[case::negative_zero("-0.0")]
fn ttl_keeps_zero(#[case] value: &str) {
    let source = format!("litellm_settings:\n  default_redis_ttl: {value}\n");
    assert_eq!(loaded(&source).default_ttl, 0);
}

#[test]
fn ttl_resolves_an_environment_reference() {
    let environment = BTreeMap::from([("REDIS_TTL".to_owned(), "45.9".to_owned())]);
    let source = "litellm_settings:\n  default_redis_ttl: os.environ/REDIS_TTL\n";
    assert_eq!(settings(source, environment).default_ttl, 45);
}

#[test]
fn ttl_reports_a_missing_environment_variable() {
    let error = failure(
        "litellm_settings:\n  default_redis_ttl: os.environ/REDIS_TTL\n",
        BTreeMap::new(),
    );
    assert!(matches!(
        error,
        ConfigError::MissingEnvironment { variable, path }
            if variable == "REDIS_TTL" && path == "litellm_settings.default_redis_ttl"
    ));
}

#[rstest]
#[case::infinity("litellm_settings:\n  default_redis_ttl: .inf\n")]
#[case::negative_infinity("litellm_settings:\n  default_redis_ttl: -.inf\n")]
#[case::nan("litellm_settings:\n  default_redis_ttl: .nan\n")]
fn ttl_rejects_a_nonfinite_value(#[case] source: &str) {
    let error = failure(source, BTreeMap::new());
    assert!(
        matches!(error, ConfigError::Yaml { .. }),
        "the YAML layer refuses a non-finite float before the section sees it"
    );
}

#[rstest]
#[case::infinity("\"inf\"")]
#[case::negative_infinity("\"-inf\"")]
#[case::nan("\"nan\"")]
fn ttl_rejects_a_text_nonfinite_value(#[case] value: &str) {
    let source = format!("litellm_settings:\n  default_redis_ttl: {value}\n");
    let error = failure(&source, BTreeMap::new());
    assert!(matches!(
        error,
        ConfigError::InvalidInteger { path, .. } if path == "litellm_settings.default_redis_ttl"
    ));
}

#[test]
fn ttl_accepts_a_text_fraction() {
    let source = "litellm_settings:\n  default_redis_ttl: \"60.5\"\n";
    assert_eq!(loaded(source).default_ttl, 60);
}

#[test]
fn ttl_does_not_depend_on_key_order() {
    let source = "litellm_settings:\n  default_redis_ttl: 90\n  cache: true\n";
    assert_eq!(
        loaded(source).default_ttl,
        90,
        "a TTL written before the switch still applies"
    );
}

#[test]
fn ttl_rejects_a_boolean() {
    let error = failure(
        "litellm_settings:\n  default_redis_ttl: true\n",
        BTreeMap::new(),
    );
    assert!(matches!(
        error,
        ConfigError::InvalidScalar { path, value } if path == "litellm_settings.default_redis_ttl" && value
    ));
}

#[test]
fn flush_size_is_none_when_unset() {
    assert_eq!(
        loaded("litellm_settings:\n  cache: true\n").redis_flush_size,
        None
    );
}

#[rstest]
#[case::integer("50", 50)]
#[case::float("50.0", 50)]
#[case::zero("0", 0)]
fn flush_size_reads_the_given_number(#[case] value: &str, #[case] expected: usize) {
    let source = format!(
        "litellm_settings:\n  cache: true\n  cache_params:\n    redis_flush_size: {value}\n"
    );
    assert_eq!(loaded(&source).redis_flush_size, Some(expected));
}

#[test]
fn flush_size_resolves_an_environment_reference() {
    let environment = BTreeMap::from([("REDIS_FLUSH".to_owned(), "25".to_owned())]);
    let source = "litellm_settings:\n  cache: true\n  cache_params:\n    redis_flush_size: os.environ/REDIS_FLUSH\n";
    assert_eq!(
        settings(source, environment).redis_flush_size,
        Some(25),
        "a plain integer reference resolves"
    );
}

#[rstest]
#[case::negative("-1")]
#[case::fraction("2.5")]
#[case::text("\"many\"")]
#[case::boolean("true")]
fn flush_size_rejects_a_bad_value(#[case] value: &str) {
    let source = format!(
        "litellm_settings:\n  cache: true\n  cache_params:\n    redis_flush_size: {value}\n"
    );
    let error = failure(&source, BTreeMap::new());
    assert!(matches!(
        error,
        ConfigError::InvalidInteger { path, .. }
            | ConfigError::InvalidScalar { path, .. }
            if path == "litellm_settings.cache_params.redis_flush_size"
    ));
}

#[test]
fn cache_switch_on_reads_both_cache_params_keys() {
    let source = "litellm_settings:\n  cache: true\n  cache_params:\n    default_redis_ttl: 120\n    redis_flush_size: 10\n";
    let settings = loaded(source);
    assert_eq!(settings.default_ttl, 120);
    assert_eq!(settings.redis_flush_size, Some(10));
}

#[test]
fn cache_switch_params_ttl_wins_over_the_litellm_settings_ttl() {
    let source = "litellm_settings:\n  cache: true\n  default_redis_ttl: 90\n  cache_params:\n    default_redis_ttl: 120\n";
    assert_eq!(loaded(source).default_ttl, 120);
}

#[test]
fn cache_switch_on_without_a_params_ttl_uses_the_litellm_settings_ttl() {
    let source = "litellm_settings:\n  cache: true\n  default_redis_ttl: 90\n  cache_params:\n    redis_flush_size: 10\n";
    assert_eq!(loaded(source).default_ttl, 90);
}

#[test]
fn cache_switch_text_is_off() {
    let source = "litellm_settings:\n  cache: \"true\"\n  default_redis_ttl: 90\n  cache_params:\n    default_redis_ttl: 120\n    redis_flush_size: 10\n";
    let settings = loaded(source);
    assert_eq!(settings.default_ttl, 90);
    assert_eq!(settings.redis_flush_size, None);
}

#[test]
fn cache_switch_off_ignores_cache_params() {
    let source = "litellm_settings:\n  cache: false\n  cache_params:\n    default_redis_ttl: 120\n    redis_flush_size: 10\n";
    let settings = loaded(source);
    assert_eq!(settings.default_ttl, 60);
    assert_eq!(settings.redis_flush_size, None);
}

#[test]
fn cache_switch_absent_ignores_cache_params() {
    let source = "litellm_settings:\n  cache_params:\n    default_redis_ttl: 120\n    redis_flush_size: 10\n";
    let settings = loaded(source);
    assert_eq!(settings.default_ttl, 60);
    assert_eq!(settings.redis_flush_size, None);
}

#[test]
fn cache_switch_on_reports_the_cache_params_path_for_a_bad_ttl() {
    let source =
        "litellm_settings:\n  cache: true\n  cache_params:\n    default_redis_ttl: \"inf\"\n";
    let error = failure(source, BTreeMap::new());
    assert!(matches!(
        error,
        ConfigError::InvalidInteger { path, .. }
            if path == "litellm_settings.cache_params.default_redis_ttl"
    ));
}
