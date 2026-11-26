#![cfg(test)]

use std::collections::BTreeMap;
use std::fs::read_to_string;
use std::sync::Arc;
use std::time::Duration;

use betterlite_env::EnvError;
use rstest::rstest;
use secrecy::ExposeSecret as _;
use tempfile::NamedTempFile;
use tracing::subscriber::set_default;

use crate::loader::ConfigError;
use crate::sections::Section as _;
use crate::sections::health_readiness::{Raw, Settings};

fn convert(yaml: &str, variables: &[(&str, &str)]) -> Result<Settings, ConfigError> {
    let environment: BTreeMap<String, String> = variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    let raw: Raw = serde_saphyr::from_str(yaml).unwrap();
    raw.convert(&environment)
}

#[rstest]
#[case::absent("general_settings: {}", false, false)]
#[case::true_booleans(
    "general_settings: {allow_public_health_readiness_details: true, enable_drain_endpoint: true}",
    true,
    true
)]
#[case::false_booleans(
    "general_settings: {allow_public_health_readiness_details: false, enable_drain_endpoint: false}",
    false,
    false
)]
#[case::strings_are_not_true(
    "general_settings: {allow_public_health_readiness_details: 'true', enable_drain_endpoint: 'yes'}",
    false,
    false
)]
#[case::numbers_are_not_true(
    "general_settings: {allow_public_health_readiness_details: 1, enable_drain_endpoint: 1}",
    false,
    false
)]
fn the_switches_are_true_only_for_a_boolean_true(
    #[case] yaml: &str,
    #[case] allow_public_details: bool,
    #[case] drain_enabled: bool,
) {
    let settings = convert(yaml, &[]).unwrap();
    assert_eq!(settings.allow_public_details, allow_public_details);
    assert_eq!(settings.drain.enabled, drain_enabled);
}

#[rstest]
#[case::absent("general_settings: {}", &[], None)]
#[case::configured("general_settings: {drain_endpoint_token: pre-stop}", &[], Some("pre-stop"))]
#[case::config_wins_over_environment(
    "general_settings: {drain_endpoint_token: configured}",
    &[("DRAIN_ENDPOINT_TOKEN", "from-env")],
    Some("configured")
)]
#[case::environment_reference(
    "general_settings: {drain_endpoint_token: os.environ/TOKEN}",
    &[("TOKEN", "from-reference")],
    Some("from-reference")
)]
#[case::empty_config_token_falls_through(
    "general_settings: {drain_endpoint_token: ''}",
    &[("DRAIN_ENDPOINT_TOKEN", "from-env")],
    Some("from-env")
)]
#[case::number_config_token_falls_through(
    "general_settings: {drain_endpoint_token: 7}",
    &[("DRAIN_ENDPOINT_TOKEN", "from-env")],
    Some("from-env")
)]
#[case::unset_reference_falls_through(
    "general_settings: {drain_endpoint_token: os.environ/MISSING}",
    &[("DRAIN_ENDPOINT_TOKEN", "from-env")],
    Some("from-env")
)]
#[case::empty_environment_is_no_token(
    "general_settings: {}",
    &[("DRAIN_ENDPOINT_TOKEN", "")],
    None
)]
fn the_drain_token_comes_from_the_config_or_the_environment(
    #[case] yaml: &str,
    #[case] variables: &[(&str, &str)],
    #[case] expected: Option<&str>,
) {
    let settings = convert(yaml, variables).unwrap();
    let token = settings
        .drain
        .token
        .as_ref()
        .map(|secret| secret.expose_secret().as_str());
    assert_eq!(token, expected);
}

#[rstest]
#[case::unset(&[], Duration::from_secs(30))]
#[case::seconds(&[("GRACEFUL_SHUTDOWN_TIMEOUT", "12.5")], Duration::from_millis(12_500))]
#[case::zero(&[("GRACEFUL_SHUTDOWN_TIMEOUT", "0")], Duration::ZERO)]
#[case::negative_clamps_to_zero(&[("GRACEFUL_SHUTDOWN_TIMEOUT", "-5")], Duration::ZERO)]
#[case::nan_never_expires(&[("GRACEFUL_SHUTDOWN_TIMEOUT", "nan")], Duration::MAX)]
#[case::infinite_never_expires(&[("GRACEFUL_SHUTDOWN_TIMEOUT", "inf")], Duration::MAX)]
#[case::empty_is_unset(&[("GRACEFUL_SHUTDOWN_TIMEOUT", "")], Duration::from_secs(30))]
#[case::not_a_number_uses_the_default(&[("GRACEFUL_SHUTDOWN_TIMEOUT", "soon")], Duration::from_secs(30))]
fn the_graceful_timeout_defaults_to_thirty_seconds(
    #[case] variables: &[(&str, &str)],
    #[case] expected: Duration,
) {
    let settings = convert("general_settings: {}", variables).unwrap();
    assert_eq!(settings.graceful_shutdown_timeout, expected);
}

#[test]
fn a_graceful_timeout_that_is_not_a_number_warns_with_the_raw_value() {
    let log = NamedTempFile::new().unwrap();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_writer(Arc::new(log.reopen().unwrap()))
        .finish();
    let guard = set_default(subscriber);
    let settings = convert(
        "general_settings: {}",
        &[("GRACEFUL_SHUTDOWN_TIMEOUT", "soon")],
    )
    .unwrap();
    drop(guard);
    assert_eq!(settings.graceful_shutdown_timeout, Duration::from_secs(30));
    let written = read_to_string(log.path()).unwrap();
    assert!(written.contains("GRACEFUL_SHUTDOWN_TIMEOUT"), "{written}");
    assert!(written.contains("soon"), "{written}");
}

#[rstest]
#[case::unset(&[], 30.0)]
#[case::value(&[("PROXY_DB_LOOKUP_STALL_WINDOW_SECONDS", "5.5")], 5.5)]
#[case::zero(&[("PROXY_DB_LOOKUP_STALL_WINDOW_SECONDS", "0")], 0.0)]
#[case::negative_clamps_to_zero(&[("PROXY_DB_LOOKUP_STALL_WINDOW_SECONDS", "-2")], 0.0)]
#[case::empty_is_unset(&[("PROXY_DB_LOOKUP_STALL_WINDOW_SECONDS", "")], 30.0)]
fn the_stall_window_defaults_and_never_goes_negative(
    #[case] variables: &[(&str, &str)],
    #[case] expected: f64,
) {
    let settings = convert("general_settings: {}", variables).unwrap();
    assert_eq!(settings.db_lookup_stall_window_seconds, expected);
}

#[test]
fn a_stall_window_that_is_not_a_number_names_the_variable() {
    let error = convert(
        "general_settings: {}",
        &[("PROXY_DB_LOOKUP_STALL_WINDOW_SECONDS", "wide")],
    )
    .unwrap_err();
    assert!(matches!(
        error,
        ConfigError::Environment {
            source: EnvError::Number { name }
        } if name == "PROXY_DB_LOOKUP_STALL_WINDOW_SECONDS"
    ));
}

#[rstest]
#[case::unset(&[], false)]
#[case::true_value(&[("LITELLM_DISABLE_NO_REDIS_WARNING", "true")], true)]
#[case::false_value(&[("LITELLM_DISABLE_NO_REDIS_WARNING", "false")], false)]
#[case::empty_is_unset(&[("LITELLM_DISABLE_NO_REDIS_WARNING", "")], false)]
fn the_no_redis_warning_flag_reads_the_environment(
    #[case] variables: &[(&str, &str)],
    #[case] expected: bool,
) {
    let settings = convert("general_settings: {}", variables).unwrap();
    assert_eq!(settings.disable_no_redis_warning, expected);
}

#[test]
fn a_no_redis_flag_that_is_not_a_boolean_names_the_variable() {
    let error = convert(
        "general_settings: {}",
        &[("LITELLM_DISABLE_NO_REDIS_WARNING", "quiet")],
    )
    .unwrap_err();
    assert!(matches!(
        error,
        ConfigError::Environment {
            source: EnvError::Bool { name }
        } if name == "LITELLM_DISABLE_NO_REDIS_WARNING"
    ));
}

#[rstest]
#[case::default("litellm_settings: {}", &[], 6000.0)]
#[case::configured("litellm_settings: {request_timeout: 1200}", &[], 1200.0)]
#[case::environment("litellm_settings: {}", &[("REQUEST_TIMEOUT", "45")], 45.0)]
#[case::config_wins("litellm_settings: {request_timeout: 1200}", &[("REQUEST_TIMEOUT", "45")], 1200.0)]
#[case::reference(
    "litellm_settings: {request_timeout: os.environ/TIMEOUT}",
    &[("TIMEOUT", "90")],
    90.0
)]
fn the_request_timeout_resolves_like_the_router_section(
    #[case] yaml: &str,
    #[case] variables: &[(&str, &str)],
    #[case] expected: f64,
) {
    let settings = convert(yaml, variables).unwrap();
    assert_eq!(settings.request_timeout, expected);
}

#[test]
fn a_request_timeout_that_is_not_a_number_names_the_path() {
    let error = convert("litellm_settings: {request_timeout: soon}", &[]).unwrap_err();
    assert!(matches!(
        error,
        ConfigError::InvalidFloat { path, .. } if path == "litellm_settings.request_timeout"
    ));
}

#[rstest]
#[case::absent("general_settings: {}", None)]
#[case::list("general_settings: {alerting: [slack, email]}", Some(&["slack", "email"][..]))]
#[case::empty_list("general_settings: {alerting: []}", Some(&[][..]))]
#[case::null("general_settings: {alerting: null}", None)]
fn the_alerting_setting_keeps_its_list(#[case] yaml: &str, #[case] expected: Option<&[&str]>) {
    let settings = convert(yaml, &[]).unwrap();
    let expected = expected.map(|items| {
        items
            .iter()
            .map(|item| (*item).to_owned())
            .collect::<Vec<String>>()
    });
    assert_eq!(settings.alerting, expected);
}
