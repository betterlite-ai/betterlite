#![cfg(test)]
use std::collections::BTreeMap;

use betterlite_env::EnvError;

use crate::loader::ConfigError;

use super::load_settings;

const DEFAULTS: &str =
    include_str!("../../../fuzz/corpus/config_yaml/realtime-probe-settings.yaml");
const DEFAULT_MAX_MESSAGE_SIZE_BYTES: usize = 16 * 1024 * 1024;

fn lookup(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

#[test]
fn loads_realtime_probe_settings() {
    let defaults = load_settings(DEFAULTS, lookup(&[])).unwrap();
    assert_eq!(defaults.connection.azure_realtime_protocol.as_deref(), None);
    assert_eq!(defaults.connection.credential_timeout_seconds, 20.0);
    assert_eq!(
        defaults.max_message_size_bytes,
        DEFAULT_MAX_MESSAGE_SIZE_BYTES
    );

    let explicit = load_settings(
        DEFAULTS,
        lookup(&[
            ("LITELLM_AZURE_REALTIME_PROTOCOL", "beta"),
            ("REALTIME_CREDENTIAL_RESOLUTION_TIMEOUT_SECONDS", "5.5"),
            ("REALTIME_WEBSOCKET_MAX_MESSAGE_SIZE_BYTES", "1024"),
        ]),
    )
    .unwrap();
    assert_eq!(
        explicit.connection.azure_realtime_protocol.as_deref(),
        Some("beta")
    );
    assert_eq!(explicit.connection.credential_timeout_seconds, 5.5);
    assert_eq!(explicit.max_message_size_bytes, 1024);
}

#[test]
fn empty_variables_count_as_unset() {
    let empty = load_settings(
        DEFAULTS,
        lookup(&[
            ("LITELLM_AZURE_REALTIME_PROTOCOL", ""),
            ("REALTIME_WEBSOCKET_MAX_MESSAGE_SIZE_BYTES", ""),
        ]),
    )
    .unwrap();
    assert_eq!(empty.connection.azure_realtime_protocol, None);
    assert_eq!(empty.max_message_size_bytes, DEFAULT_MAX_MESSAGE_SIZE_BYTES);
}

#[test]
fn a_malformed_size_is_an_error() {
    let error = load_settings(
        DEFAULTS,
        lookup(&[("REALTIME_WEBSOCKET_MAX_MESSAGE_SIZE_BYTES", "not-a-number")]),
    )
    .unwrap_err();
    assert!(matches!(
        error,
        ConfigError::Environment {
            source: EnvError::Number { .. }
        }
    ));
}
