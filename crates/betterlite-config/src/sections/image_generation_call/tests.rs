#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;
use secrecy::ExposeSecret as _;

use super::Settings;
use crate::loader::ConfigLoader;

fn settings(source: &str, variables: &[(&str, &str)]) -> Result<Settings, String> {
    let lookup: BTreeMap<String, String> = variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    ConfigLoader { lookup }
        .load(source)
        .map(|config| config.sections.image_generation_call)
        .map_err(|error| error.to_string())
}

// each fallback tier of the Azure and Foundry chains, and the default that names no provider key
#[rstest]
#[case::unset(&[], None, None, None, None, None, "azure")]
#[case::azure_api_base(&[("AZURE_API_BASE", "https://env.example")], Some("https://env.example"), None, None, None, None, "azure")]
#[case::azure_api_version(&[("AZURE_API_VERSION", "2024-02-01")], None, Some("2024-02-01"), None, None, None, "azure")]
#[case::azure_openai_key(&[("AZURE_OPENAI_API_KEY", "openai-env")], None, None, Some("openai-env"), None, None, "azure")]
#[case::azure_api_key(&[("AZURE_API_KEY", "azure-env")], None, None, None, Some("azure-env"), None, "azure")]
#[case::azure_ad_token(&[("AZURE_AD_TOKEN", "token-env")], None, None, None, None, Some("token-env"), "azure")]
#[case::azure_api_type(&[("AZURE_API_TYPE", "azure_ad")], None, None, None, None, None, "azure_ad")]
#[case::empty_is_unset(
    &[
        ("AZURE_API_BASE", ""),
        ("AZURE_API_VERSION", ""),
        ("AZURE_OPENAI_API_KEY", ""),
        ("AZURE_API_KEY", ""),
        ("AZURE_AD_TOKEN", ""),
        ("AZURE_API_TYPE", ""),
    ],
    None,
    None,
    None,
    None,
    None,
    "azure"
)]
fn loads_settings(
    #[case] variables: &[(&str, &str)],
    #[case] api_base: Option<&str>,
    #[case] api_version: Option<&str>,
    #[case] openai_key: Option<&str>,
    #[case] azure_key: Option<&str>,
    #[case] ad_token: Option<&str>,
    #[case] api_type: &str,
) {
    let settings = settings("litellm_settings: {}\n", variables).expect("settings load");
    assert_eq!(settings.azure_api_base.as_deref(), api_base);
    assert_eq!(settings.azure_api_version.as_deref(), api_version);
    assert_eq!(
        settings
            .azure_openai_api_key
            .as_ref()
            .map(|key| key.expose_secret()),
        openai_key
    );
    assert_eq!(
        settings
            .azure_api_key
            .as_ref()
            .map(|key| key.expose_secret()),
        azure_key
    );
    assert_eq!(
        settings
            .azure_ad_token
            .as_ref()
            .map(|key| key.expose_secret()),
        ad_token
    );
    assert_eq!(settings.azure_api_type, api_type);
}

#[test]
fn loads_module_globals() {
    let source = "litellm_settings:\n  api_base: https://global.example\n  api_version: 2024-06-01\n  api_key: global-key\n  azure_key: global-azure-key\n";
    let settings = settings(source, &[]).expect("settings load");
    assert_eq!(settings.api_base.as_deref(), Some("https://global.example"));
    assert_eq!(settings.api_version.as_deref(), Some("2024-06-01"));
    assert_eq!(
        settings.api_key.expect("api key").expose_secret(),
        "global-key"
    );
    assert_eq!(
        settings.azure_key.expect("azure key").expose_secret(),
        "global-azure-key"
    );
}

#[test]
fn loads_foundry_environment() {
    let variables = [
        ("AZURE_AI_API_BASE", "https://foundry.example"),
        ("AZURE_AI_API_KEY", "foundry-key"),
    ];
    let settings = settings("litellm_settings: {}\n", &variables).expect("settings load");
    assert_eq!(
        settings.azure_ai_api_base.as_deref(),
        Some("https://foundry.example")
    );
    assert_eq!(
        settings
            .azure_ai_api_key
            .expect("foundry key")
            .expose_secret(),
        "foundry-key"
    );
}

#[test]
fn the_shipped_seed_loads() {
    let source = include_str!("../../../fuzz/corpus/config_yaml/image-generation-call-valid.yaml");
    let settings = settings(source, &[]).expect("seed load");
    assert_eq!(settings.api_base.as_deref(), Some("https://seed.example"));
    assert_eq!(settings.api_version.as_deref(), Some("2024-02-01"));
    assert_eq!(
        settings.azure_key.expect("seed azure key").expose_secret(),
        "seed-azure-key"
    );
}
