#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::azure_endpoints::AZURE_DEFAULT_API_VERSION;
use betterlite_config::types::Config;
use rstest::rstest;
use secrecy::ExposeSecret as _;

use super::Slot;

fn load(source: &str, environment: &BTreeMap<String, String>) -> Config {
    ConfigLoader {
        lookup: environment.clone(),
    }
    .load(source)
    .expect("config")
}

// the proxy sets each litellm_settings key as a litellm global, and the adapter reads those before the environment
#[test]
fn the_azure_slot_reads_the_litellm_settings_globals() {
    let config = load(
        "litellm_settings: {api_base: 'https://settings.example', api_version: '2024-10-21', api_key: settings-key, azure_key: settings-azure-key, headers: {x-gateway: gateway-value}}\n",
        &BTreeMap::new(),
    );
    let slot = Slot::new(&config);
    let globals = &slot.composition.globals;
    assert_eq!(
        globals.api_base.as_deref(),
        Some("https://settings.example")
    );
    assert_eq!(globals.api_version.as_deref(), Some("2024-10-21"));
    assert_eq!(
        globals
            .api_key
            .as_ref()
            .expect("the settings api key")
            .expose_secret(),
        "settings-key"
    );
    assert_eq!(
        globals
            .azure_key
            .as_ref()
            .expect("the settings azure key")
            .expose_secret(),
        "settings-azure-key"
    );
    assert_eq!(
        globals
            .headers
            .as_ref()
            .and_then(|headers| headers.get("x-gateway"))
            .map(String::as_str),
        Some("gateway-value")
    );
}

// the environment the config loaded with is the whole environment the adapter sees, captured here at startup
#[rstest]
#[case::api_base("AZURE_API_BASE", "https://environment.example")]
#[case::api_version("AZURE_API_VERSION", "2024-06-01")]
#[case::api_key("AZURE_API_KEY", "environment-key")]
#[case::openai_api_key("AZURE_OPENAI_API_KEY", "openai-environment-key")]
#[case::ad_token("AZURE_AD_TOKEN", "ad-token")]
#[case::api_type("AZURE_API_TYPE", "azure_ad")]
fn the_azure_slot_carries_the_startup_environment(#[case] name: &str, #[case] expected: &str) {
    let environment = BTreeMap::from([
        (
            String::from("AZURE_API_BASE"),
            String::from("https://environment.example"),
        ),
        (
            String::from("AZURE_API_VERSION"),
            String::from("2024-06-01"),
        ),
        (
            String::from("AZURE_API_KEY"),
            String::from("environment-key"),
        ),
        (
            String::from("AZURE_OPENAI_API_KEY"),
            String::from("openai-environment-key"),
        ),
        (String::from("AZURE_AD_TOKEN"), String::from("ad-token")),
        (String::from("AZURE_API_TYPE"), String::from("azure_ad")),
    ]);
    let config = load("", &environment);
    let slot = Slot::new(&config);
    assert_eq!(
        slot.composition.secret(name).expect(name).expose_secret(),
        expected,
        "{name}"
    );
}

// AZURE_DEFAULT_API_VERSION stays with the endpoints section, which already resolved the key and its default
#[test]
fn the_azure_slot_takes_the_default_api_version_from_the_endpoints_section() {
    let configured = load(
        "litellm_settings: {AZURE_DEFAULT_API_VERSION: '2025-04-01-preview'}\n",
        &BTreeMap::new(),
    );
    assert_eq!(
        Slot::new(&configured)
            .composition
            .globals
            .azure_default_api_version
            .as_deref(),
        Some("2025-04-01-preview")
    );
    let defaults = load("", &BTreeMap::new());
    assert_eq!(
        Slot::new(&defaults)
            .composition
            .globals
            .azure_default_api_version
            .as_deref(),
        Some(AZURE_DEFAULT_API_VERSION)
    );
}
