use crate::env_integer::timeout::TimeoutError;

use super::super::{RealtimeHttpCallSettings, SettingsError, load_settings};
use super::{CONFIGURED, DEFAULT_REQUEST_TIMEOUT_SECONDS, DEFAULTS, exposed, lookup};

#[test]
fn defaults_to_no_globals_no_environment_and_the_request_timeout() {
    let settings = load_settings(DEFAULTS, lookup(&[])).unwrap();
    assert_eq!(settings.api_base, None);
    assert_eq!(exposed(settings.api_key.as_ref()), None);
    assert_eq!(exposed(settings.openai_key.as_ref()), None);
    assert_eq!(settings.openai_api_base, None);
    assert_eq!(exposed(settings.openai_api_key.as_ref()), None);
    assert_eq!(settings.azure_api_base, None);
    assert_eq!(exposed(settings.azure_api_key.as_ref()), None);
    assert_eq!(settings.azure_api_version, None);
    assert_eq!(
        settings.request_timeout_seconds.to_bits(),
        DEFAULT_REQUEST_TIMEOUT_SECONDS.to_bits()
    );
}

#[test]
fn reads_the_globals_environment_and_timeout() {
    let settings = load_settings(
        CONFIGURED,
        lookup(&[
            ("OPENAI_API_BASE", "https://openai.example/v1"),
            ("OPENAI_API_KEY", "sk-env-openai"),
            ("AZURE_API_BASE", "https://azure.example"),
            ("AZURE_API_KEY", "azure-key"),
            ("AZURE_API_VERSION", "2024-08-01"),
            ("REQUEST_TIMEOUT", "12.5"),
        ]),
    )
    .unwrap();
    assert_globals(&settings);
    assert_environment(&settings);
    assert_eq!(
        settings.request_timeout_seconds.to_bits(),
        12.5_f64.to_bits()
    );
}

fn assert_globals(settings: &RealtimeHttpCallSettings) {
    assert_eq!(
        settings.api_base.as_deref(),
        Some("https://global.example/v1")
    );
    assert_eq!(
        exposed(settings.api_key.as_ref()),
        Some("sk-global-realtime")
    );
    assert_eq!(
        exposed(settings.openai_key.as_ref()),
        Some("sk-openai-global")
    );
}

fn assert_environment(settings: &RealtimeHttpCallSettings) {
    assert_eq!(
        settings.openai_api_base.as_deref(),
        Some("https://openai.example/v1")
    );
    assert_eq!(
        exposed(settings.openai_api_key.as_ref()),
        Some("sk-env-openai")
    );
    assert_eq!(
        settings.azure_api_base.as_deref(),
        Some("https://azure.example")
    );
    assert_eq!(exposed(settings.azure_api_key.as_ref()), Some("azure-key"));
    assert_eq!(settings.azure_api_version.as_deref(), Some("2024-08-01"));
}

#[test]
fn treats_an_empty_value_as_unset() {
    let settings = load_settings(
        CONFIGURED,
        lookup(&[
            ("OPENAI_API_BASE", ""),
            ("OPENAI_API_KEY", ""),
            ("AZURE_API_BASE", ""),
            ("AZURE_API_KEY", ""),
            ("AZURE_API_VERSION", ""),
            ("REQUEST_TIMEOUT", ""),
        ]),
    )
    .unwrap();
    assert_eq!(settings.openai_api_base, None);
    assert_eq!(exposed(settings.openai_api_key.as_ref()), None);
    assert_eq!(settings.azure_api_base, None);
    assert_eq!(exposed(settings.azure_api_key.as_ref()), None);
    assert_eq!(settings.azure_api_version, None);
    assert_eq!(
        settings.request_timeout_seconds.to_bits(),
        DEFAULT_REQUEST_TIMEOUT_SECONDS.to_bits()
    );
}

#[test]
fn rejects_a_non_numeric_timeout() {
    let error = load_settings(DEFAULTS, lookup(&[("REQUEST_TIMEOUT", "twenty")])).unwrap_err();
    assert!(matches!(
        error,
        SettingsError::Timeout(TimeoutError::Invalid { .. })
    ));
}
