#![cfg(test)]
use std::collections::BTreeMap;

use betterlite_env::EnvError;
use betterlite_models::decisions::Json;
use secrecy::{ExposeSecret, SecretString};

use crate::loader::ConfigError;

use super::{
    AZURE_PROTOCOL_VARIABLE, CREDENTIAL_TIMEOUT_VARIABLE, DEFAULT_TIMEOUT_SECONDS,
    RealtimeConnectionSettings, default_credential_timeout, load_settings,
};

const DEFAULTS: &str =
    include_str!("../../../fuzz/corpus/config_yaml/realtime-connect-default.yaml");
const CONFIGURED: &str =
    include_str!("../../../fuzz/corpus/config_yaml/realtime-connect-configured.yaml");
// the vertex credential section validates VERTEXAI_CREDENTIALS, so it must decode as a service account
const SERVICE_ACCOUNT: &str =
    r#"{"type":"service_account","private_key":"KEYMATERIAL","client_email":"sa@example.com"}"#;

fn lookup(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn exposed(value: Option<&SecretString>) -> Option<&str> {
    value.map(ExposeSecret::expose_secret)
}

#[test]
fn loads_realtime_connection_settings() {
    let defaults = load_settings(DEFAULTS, lookup(&[])).unwrap();
    assert_no_default_globals(&defaults);
    assert_no_default_environment_values(&defaults);
    assert_default_credential_timeout(&defaults);
}

fn assert_no_default_globals(defaults: &RealtimeConnectionSettings) {
    assert_eq!(defaults.api_base, None);
    assert_eq!(exposed(defaults.api_key.as_ref()), None);
    assert_eq!(exposed(defaults.openai_key.as_ref()), None);
    assert_eq!(exposed(defaults.xai_key.as_ref()), None);
    assert_eq!(defaults.vertex_project, None);
    assert_eq!(defaults.vertex_location, None);
}

fn assert_no_default_environment_values(defaults: &RealtimeConnectionSettings) {
    assert_eq!(defaults.azure_api_base, None);
    assert_eq!(exposed(defaults.azure_api_key.as_ref()), None);
    assert_eq!(exposed(defaults.openai_api_key.as_ref()), None);
    assert_eq!(defaults.xai_api_base, None);
    assert_eq!(exposed(defaults.xai_api_key.as_ref()), None);
    assert_eq!(defaults.vertexai_credentials, None);
    assert_eq!(defaults.vertexai_project, None);
    assert_eq!(defaults.vertexai_location, None);
    assert_eq!(defaults.azure_realtime_protocol, None);
}

fn assert_default_credential_timeout(defaults: &RealtimeConnectionSettings) {
    assert_eq!(
        defaults.credential_timeout_seconds.to_bits(),
        DEFAULT_TIMEOUT_SECONDS.to_bits()
    );
    assert_eq!(
        default_credential_timeout().to_bits(),
        DEFAULT_TIMEOUT_SECONDS.to_bits()
    );
}

#[test]
fn loads_realtime_connection_settings_from_the_config_and_environment() {
    let configured = configured_settings();
    assert_configured_file_values(&configured);
    assert_configured_environment_values(&configured);
}

fn configured_settings() -> RealtimeConnectionSettings {
    load_settings(
        CONFIGURED,
        lookup(&[
            ("AZURE_API_BASE", "https://azure.example"),
            ("AZURE_API_KEY", "azure-key"),
            ("OPENAI_API_KEY", "openai-key"),
            ("XAI_API_BASE", "https://xai.example/v1"),
            ("XAI_API_KEY", "xai-key"),
            ("VERTEXAI_CREDENTIALS", SERVICE_ACCOUNT),
            ("VERTEXAI_PROJECT", "env-project"),
            ("VERTEXAI_LOCATION", "env-region"),
            (AZURE_PROTOCOL_VARIABLE, "GA"),
            (CREDENTIAL_TIMEOUT_VARIABLE, "12.5"),
        ]),
    )
    .unwrap()
}

fn assert_configured_file_values(configured: &RealtimeConnectionSettings) {
    assert_eq!(
        configured.api_base.as_deref(),
        Some("https://global.example/v1")
    );
    assert_eq!(
        exposed(configured.api_key.as_ref()),
        Some("sk-global-realtime")
    );
    assert_eq!(
        exposed(configured.openai_key.as_ref()),
        Some("sk-openai-global")
    );
    assert_eq!(exposed(configured.xai_key.as_ref()), Some("sk-xai-global"));
    assert_eq!(
        configured.vertex_project.as_deref(),
        Some("global-vertex-project")
    );
    assert_eq!(
        configured.vertex_location.as_deref(),
        Some("global-vertex-region")
    );
}

fn assert_configured_environment_values(configured: &RealtimeConnectionSettings) {
    assert_eq!(
        configured.azure_api_base.as_deref(),
        Some("https://azure.example")
    );
    assert_eq!(
        exposed(configured.azure_api_key.as_ref()),
        Some("azure-key")
    );
    assert_eq!(
        exposed(configured.openai_api_key.as_ref()),
        Some("openai-key")
    );
    assert_eq!(
        configured.xai_api_base.as_deref(),
        Some("https://xai.example/v1")
    );
    assert_eq!(exposed(configured.xai_api_key.as_ref()), Some("xai-key"));
    assert_eq!(
        configured.vertexai_credentials,
        Some(Json::Text(String::from(SERVICE_ACCOUNT)))
    );
    assert_eq!(configured.vertexai_project.as_deref(), Some("env-project"));
    assert_eq!(configured.vertexai_location.as_deref(), Some("env-region"));
    assert_eq!(configured.azure_realtime_protocol.as_deref(), Some("GA"));
    assert_eq!(
        configured.credential_timeout_seconds.to_bits(),
        12.5_f64.to_bits()
    );
}

#[test]
fn loads_realtime_connection_settings_treats_empty_environment_values_as_unset() {
    let empty = load_settings(
        DEFAULTS,
        lookup(&[
            ("AZURE_API_BASE", ""),
            ("AZURE_API_KEY", ""),
            ("OPENAI_API_KEY", ""),
            ("XAI_API_BASE", ""),
            ("XAI_API_KEY", ""),
            ("VERTEXAI_CREDENTIALS", ""),
            ("VERTEXAI_PROJECT", ""),
            ("VERTEXAI_LOCATION", ""),
            (AZURE_PROTOCOL_VARIABLE, ""),
            (CREDENTIAL_TIMEOUT_VARIABLE, ""),
        ]),
    )
    .unwrap();
    assert_eq!(empty.azure_api_base, None);
    assert_eq!(exposed(empty.azure_api_key.as_ref()), None);
    assert_eq!(exposed(empty.openai_api_key.as_ref()), None);
    assert_eq!(empty.xai_api_base, None);
    assert_eq!(exposed(empty.xai_api_key.as_ref()), None);
    assert_eq!(empty.vertexai_credentials, None);
    assert_eq!(empty.vertexai_project, None);
    assert_eq!(empty.vertexai_location, None);
    assert_eq!(empty.azure_realtime_protocol, None);
    assert_eq!(
        empty.credential_timeout_seconds.to_bits(),
        DEFAULT_TIMEOUT_SECONDS.to_bits()
    );
}

#[test]
fn loads_realtime_connection_settings_rejects_a_non_numeric_timeout() {
    let error =
        load_settings(DEFAULTS, lookup(&[(CREDENTIAL_TIMEOUT_VARIABLE, "twenty")])).unwrap_err();
    let ConfigError::Environment {
        source: EnvError::Number { name },
    } = error
    else {
        panic!("expected an environment number error");
    };
    assert_eq!(name, CREDENTIAL_TIMEOUT_VARIABLE);
}
