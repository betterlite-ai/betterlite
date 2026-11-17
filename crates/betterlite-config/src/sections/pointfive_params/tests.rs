#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use crate::loader::{ConfigError, ConfigLoader};
use crate::sections::pointfive_params::{
    DEFAULT_BATCH_SIZE, DEFAULT_FLUSH_INTERVAL, DEFAULT_MAX_BATCH_BYTES,
    DEFAULT_MAX_UPLOAD_RETRIES, PointFiveInitParams, Settings,
};

fn environment(variables: &[(&str, &str)]) -> BTreeMap<String, String> {
    variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn load(source: &str) -> Result<Settings, ConfigError> {
    ConfigLoader {
        lookup: environment(&[]),
    }
    .load(source)
    .map(|config| config.sections.pointfive_params)
}

fn loaded(source: &str) -> Settings {
    load(source).expect("the config loads")
}

fn params(source: &str) -> PointFiveInitParams {
    *loaded(source).params.expect("the params are set")
}

fn defaults() -> PointFiveInitParams {
    PointFiveInitParams {
        api_key: None,
        api_url: None,
        batch_size: DEFAULT_BATCH_SIZE,
        flush_interval: DEFAULT_FLUSH_INTERVAL,
        max_batch_bytes: DEFAULT_MAX_BATCH_BYTES,
        max_upload_retries: DEFAULT_MAX_UPLOAD_RETRIES,
        turn_off_message_logging: false,
    }
}

#[test]
fn a_missing_key_leaves_the_params_unset() {
    assert!(loaded("{}").params.is_none());
    assert!(loaded("litellm_settings: {}").params.is_none());
}

#[test]
fn a_null_key_leaves_the_params_unset() {
    assert!(
        loaded("litellm_settings: {pointfive_params: null}")
            .params
            .is_none()
    );
}

#[test]
fn an_empty_mapping_takes_every_default() {
    let params = params("litellm_settings: {pointfive_params: {}}");
    assert!(params.api_key.is_none());
    assert!(params.api_url.is_none());
    assert_eq!(params.batch_size, 1_000);
    assert_eq!(params.flush_interval, 300);
    assert_eq!(params.max_batch_bytes, 8 * 1024 * 1024);
    assert_eq!(params.max_upload_retries, 3);
    assert!(!params.turn_off_message_logging);
    assert_eq!(params.batch_size, defaults().batch_size);
}

#[test]
fn every_key_is_read() {
    let params = params(
        "litellm_settings:\n  pointfive_params:\n    api_key: sk-test\n    api_url: https://pointfive.test/ingestion\n    batch_size: 25\n    flush_interval: 60\n    max_batch_bytes: 4096\n    max_upload_retries: 7\n    turn_off_message_logging: true\n",
    );
    assert_eq!(
        params
            .api_key
            .as_ref()
            .map(secrecy::ExposeSecret::expose_secret),
        Some("sk-test")
    );
    assert_eq!(
        params.api_url.as_deref(),
        Some("https://pointfive.test/ingestion")
    );
    assert_eq!(params.batch_size, 25);
    assert_eq!(params.flush_interval, 60);
    assert_eq!(params.max_batch_bytes, 4_096);
    assert_eq!(params.max_upload_retries, 7);
    assert!(params.turn_off_message_logging);
}

#[test]
fn an_environment_reference_stays_a_reference() {
    let params = params(
        "litellm_settings:\n  pointfive_params:\n    api_key: os.environ/POINTFIVE_API_KEY\n",
    );
    assert_eq!(
        params
            .api_key
            .as_ref()
            .map(secrecy::ExposeSecret::expose_secret),
        Some("os.environ/POINTFIVE_API_KEY")
    );
}

#[test]
fn nulls_take_the_defaults() {
    let params = params(
        "litellm_settings:\n  pointfive_params:\n    api_key: null\n    api_url: null\n    batch_size: null\n    flush_interval: null\n    max_batch_bytes: null\n    max_upload_retries: null\n    turn_off_message_logging: null\n",
    );
    assert!(params.api_key.is_none());
    assert!(params.api_url.is_none());
    assert_eq!(params.batch_size, 1_000);
    assert_eq!(params.flush_interval, 300);
    assert_eq!(params.max_batch_bytes, 8 * 1024 * 1024);
    assert_eq!(params.max_upload_retries, 3);
    assert!(!params.turn_off_message_logging);
}

#[test]
fn unknown_keys_are_ignored_like_pydantic() {
    let params = params(
        "litellm_settings:\n  pointfive_params:\n    batch_size: 5\n    message_logging: true\n    nested:\n      deep: value\n",
    );
    assert_eq!(params.batch_size, 5);
    assert_eq!(params.flush_interval, 300);
}

#[rstest]
#[case::batch_size("batch_size: 0", "batch_size")]
#[case::flush_interval("flush_interval: 0", "flush_interval")]
#[case::max_batch_bytes("max_batch_bytes: 0", "max_batch_bytes")]
#[case::max_upload_retries("max_upload_retries: 0", "max_upload_retries")]
fn zero_is_refused(#[case] line: &str, #[case] field: &str) {
    let source = format!("litellm_settings:\n  pointfive_params:\n    {line}\n");
    let error = load(&source).expect_err("zero is refused");
    assert!(error.to_string().contains(field), "{error}");
}

#[rstest]
#[case::batch_size("batch_size: many")]
#[case::flush_interval("flush_interval: 1.5")]
#[case::turn_off_message_logging("turn_off_message_logging: sometimes")]
fn a_value_of_the_wrong_type_fails_the_load(#[case] line: &str) {
    let source = format!("litellm_settings:\n  pointfive_params:\n    {line}\n");
    let error = load(&source).expect_err("the value is refused");
    assert!(matches!(error, ConfigError::Yaml { .. }), "{error}");
}

#[test]
fn a_negative_count_is_refused() {
    let error = load("litellm_settings:\n  pointfive_params:\n    batch_size: -1\n")
        .expect_err("a negative count is refused");
    assert!(matches!(error, ConfigError::Yaml { .. }), "{error}");
}

#[test]
fn the_defaults_match_litellm() {
    let params = defaults();
    assert_eq!(params.batch_size, 1_000);
    assert_eq!(params.flush_interval, 300);
    assert_eq!(params.max_batch_bytes, 8 * 1024 * 1024);
    assert_eq!(params.max_upload_retries, 3);
}
