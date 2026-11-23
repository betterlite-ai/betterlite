#![cfg(test)]

use std::collections::BTreeMap;

use secrecy::ExposeSecret as _;

use crate::loader::{ConfigError, ConfigLoader};

use super::{
    AD_TOKEN_VARIABLE, API_BASE_VARIABLE, API_KEY_VARIABLE, API_TYPE_VARIABLE,
    API_VERSION_VARIABLE, OPENAI_API_KEY_VARIABLE, Settings,
};

fn load(source: &str, environment: &[(&str, &str)]) -> Result<Settings, ConfigError> {
    let lookup = environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect::<BTreeMap<_, _>>();
    ConfigLoader { lookup }
        .load(source)
        .map(|config| config.sections.azure_provider_globals)
}

fn settings(params: &str) -> String {
    format!("litellm_settings: {params}\n")
}

// the proxy sets each litellm_settings key as a litellm global, and the Azure adapter reads them before the environment
#[test]
fn azure_provider_globals_come_from_litellm_settings() {
    let loaded = load(
        &settings(
            "{api_base: 'https://settings.example', api_version: '2024-10-21', api_key: os.environ/GLOBAL_API_KEY, azure_key: settings-azure-key, headers: {x-gateway: os.environ/GATEWAY_VALUE}}",
        ),
        &[("GLOBAL_API_KEY", "settings-key"), ("GATEWAY_VALUE", "gateway-value")],
    )
    .expect("the config should load");
    assert_eq!(
        loaded.globals.api_base.as_deref(),
        Some("https://settings.example")
    );
    assert_eq!(loaded.globals.api_version.as_deref(), Some("2024-10-21"));
    let api_key = loaded
        .globals
        .api_key
        .as_ref()
        .expect("the settings carry the api key");
    assert_eq!(api_key.expose_secret(), "settings-key");
    let azure_key = loaded
        .globals
        .azure_key
        .as_ref()
        .expect("the settings carry the azure key");
    assert_eq!(azure_key.expose_secret(), "settings-azure-key");
    let headers = loaded.globals.headers.as_ref().expect("the headers map");
    assert_eq!(
        headers.get("x-gateway").map(String::as_str),
        Some("gateway-value")
    );
}

// every Azure adapter variable is captured at config load, so no request reads the process environment
#[test]
fn azure_provider_environment_variables_are_read_at_load() {
    let loaded = load(
        "",
        &[
            (API_BASE_VARIABLE, "https://environment.example"),
            (API_VERSION_VARIABLE, "2024-06-01"),
            (API_KEY_VARIABLE, "environment-key"),
            (OPENAI_API_KEY_VARIABLE, "openai-environment-key"),
            (AD_TOKEN_VARIABLE, "ad-token"),
            (API_TYPE_VARIABLE, "azure_ad"),
        ],
    )
    .expect("the config should load");
    let environment = &loaded.environment;
    assert_eq!(
        environment.get(API_BASE_VARIABLE).map(String::as_str),
        Some("https://environment.example")
    );
    assert_eq!(
        environment.get(API_VERSION_VARIABLE).map(String::as_str),
        Some("2024-06-01")
    );
    assert_eq!(
        environment.get(API_KEY_VARIABLE).map(String::as_str),
        Some("environment-key")
    );
    assert_eq!(
        environment.get(OPENAI_API_KEY_VARIABLE).map(String::as_str),
        Some("openai-environment-key")
    );
    assert_eq!(
        environment.get(AD_TOKEN_VARIABLE).map(String::as_str),
        Some("ad-token")
    );
    assert_eq!(
        environment.get(API_TYPE_VARIABLE).map(String::as_str),
        Some("azure_ad")
    );
    assert_eq!(environment.len(), 6);
}

#[test]
fn an_empty_azure_environment_value_counts_as_unset() {
    let loaded = load(
        "",
        &[
            (API_BASE_VARIABLE, ""),
            (API_KEY_VARIABLE, ""),
            (API_TYPE_VARIABLE, ""),
        ],
    )
    .expect("the config should load");
    assert!(loaded.environment.is_empty());
}

#[test]
fn azure_provider_globals_default_to_none() {
    let loaded = load("", &[]).expect("the config should load");
    assert_eq!(loaded.globals.api_base, None);
    assert_eq!(loaded.globals.api_version, None);
    assert!(loaded.globals.api_key.is_none());
    assert!(loaded.globals.azure_key.is_none());
    assert!(loaded.globals.headers.is_none());
    assert!(loaded.environment.is_empty());
}

#[test]
fn a_missing_azure_environment_reference_names_the_path() {
    let error = load(&settings("{api_base: os.environ/MISSING_AZURE_BASE}"), &[])
        .expect_err("the reference cannot resolve");
    let rendered = error.to_string();
    assert!(rendered.contains("MISSING_AZURE_BASE"), "{rendered}");
    assert!(rendered.contains("litellm_settings.api_base"), "{rendered}");
}

// the section reads only its own keys, so an unrelated litellm_settings key stays unsupported
#[test]
fn an_unrelated_litellm_settings_key_stays_unsupported() {
    let error =
        load(&settings("{azure_unrelated_setting: 1}"), &[]).expect_err("the key is unknown");
    let ConfigError::Unsupported { paths } = error else {
        panic!("expected unsupported keys, got {error:?}");
    };
    assert_eq!(paths, ["litellm_settings.azure_unrelated_setting"]);
}
