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
        .map(|config| config.sections.image_variations)
        .map_err(|error| error.to_string())
}

#[test]
fn loads_settings() {
    let variables = [
        ("OPENAI_API_KEY", "env-openai-key"),
        ("OPENAI_BASE_URL", "https://env.example/v1"),
        ("OPENAI_API_BASE", "https://legacy.example/v1"),
        ("TOPAZ_API_KEY", "env-topaz-key"),
        ("TOPAZ_API_BASE", "https://topaz.example"),
    ];
    let source = "litellm_settings:\n  api_base: https://global.example\n  api_key: global-key\n  openai_key: configured-openai-key\n";
    let settings = settings(source, &variables).expect("settings load");
    assert_eq!(settings.request_timeout_seconds, 6000.0);
    assert_eq!(settings.api_base.as_deref(), Some("https://global.example"));
    assert_eq!(
        settings.api_key.expect("api key").expose_secret(),
        "global-key"
    );
    assert_eq!(
        settings
            .openai_key
            .expect("configured OpenAI key")
            .expose_secret(),
        "configured-openai-key"
    );
    assert_eq!(
        settings.openai_base_url.as_deref(),
        Some("https://env.example/v1")
    );
    assert_eq!(
        settings.openai_api_base.as_deref(),
        Some("https://legacy.example/v1")
    );
    assert_eq!(
        settings
            .openai_api_key
            .expect("environment OpenAI key")
            .expose_secret(),
        "env-openai-key"
    );
    assert_eq!(
        settings.topaz_api_key.expect("Topaz key").expose_secret(),
        "env-topaz-key"
    );
    assert_eq!(
        settings.topaz_api_base.as_deref(),
        Some("https://topaz.example")
    );
}

#[rstest]
#[case::unset_defaults(6000.0, &[])]
#[case::environment_value(7.0, &[("REQUEST_TIMEOUT", "7")])]
#[case::fractional_environment_value(0.5, &[("REQUEST_TIMEOUT", "0.5")])]
#[case::empty_environment_is_unset(6000.0, &[("REQUEST_TIMEOUT", "")])]
fn request_timeout(#[case] expected: f64, #[case] variables: &[(&str, &str)]) {
    let settings = settings("litellm_settings: {}\n", variables).expect("settings load");
    assert_eq!(settings.request_timeout_seconds, expected);
}

#[test]
fn invalid_request_timeout_names_the_variable() {
    let error = settings("litellm_settings: {}\n", &[("REQUEST_TIMEOUT", "soon")]).unwrap_err();
    assert!(error.contains("REQUEST_TIMEOUT"), "{error}");
}

#[test]
fn the_shipped_seed_loads() {
    let source = include_str!("../../../fuzz/corpus/config_yaml/image-variations-valid.yaml");
    let settings = settings(source, &[]).expect("seed load");
    assert_eq!(settings.request_timeout_seconds, 6000.0);
    assert_eq!(settings.api_base.as_deref(), Some("https://seed.example"));
    assert_eq!(
        settings
            .openai_key
            .expect("seed OpenAI key")
            .expose_secret(),
        "seed-openai-key"
    );
}
