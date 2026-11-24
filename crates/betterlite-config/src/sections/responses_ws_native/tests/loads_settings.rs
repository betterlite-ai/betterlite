#![cfg(test)]

use std::collections::BTreeMap;

use secrecy::ExposeSecret as _;

use crate::loader::ConfigLoader;
use crate::sections::responses_ws_native::Settings;

fn load(source: &str, variables: &[(&str, &str)]) -> Result<Settings, String> {
    let lookup: BTreeMap<String, String> = variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    ConfigLoader { lookup }
        .load(source)
        .map(|config| config.sections.responses_ws_native)
        .map_err(|error| error.to_string())
}

#[test]
fn loads_settings() {
    let source = "litellm_settings:\n  api_base: https://global.test\n  api_key: sk-global\n  openai_key: sk-openai\n";
    let settings = load(source, &[("OPENAI_API_KEY", "sk-env")]).expect("settings load");
    assert_eq!(settings.api_base.as_deref(), Some("https://global.test"));
    assert_eq!(
        settings.api_key.expect("api key").expose_secret(),
        "sk-global"
    );
    assert_eq!(
        settings.openai_key.expect("openai key").expose_secret(),
        "sk-openai"
    );
    assert_eq!(
        settings.openai_api_key.expect("env key").expose_secret(),
        "sk-env"
    );
}

#[test]
fn defaults_every_unset_setting() {
    let settings = load("litellm_settings: {}\n", &[]).expect("settings load");
    assert!(settings.api_base.is_none());
    assert!(settings.api_key.is_none());
    assert!(settings.openai_key.is_none());
    assert!(settings.openai_api_key.is_none());
}

#[test]
fn resolves_environment_references() {
    let source = "litellm_settings:\n  api_base: os.environ/GLOBAL_BASE\n  api_key: os.environ/GLOBAL_KEY\n  openai_key: os.environ/OPENAI_KEY\n";
    let settings = load(
        source,
        &[
            ("GLOBAL_BASE", "https://env.test"),
            ("GLOBAL_KEY", "key-env"),
            ("OPENAI_KEY", "openai-env"),
            ("OPENAI_API_KEY", "sk-env"),
        ],
    )
    .expect("settings load");
    assert_eq!(settings.api_base.as_deref(), Some("https://env.test"));
    assert_eq!(
        settings.api_key.expect("api key").expose_secret(),
        "key-env"
    );
    assert_eq!(
        settings.openai_key.expect("openai key").expose_secret(),
        "openai-env"
    );
    assert_eq!(
        settings.openai_api_key.expect("env key").expose_secret(),
        "sk-env"
    );
}

#[test]
fn reports_a_missing_environment_reference() {
    let error = load(
        "litellm_settings:\n  api_key: os.environ/MISSING_KEY\n",
        &[],
    )
    .expect_err("the reference is missing");
    assert!(error.contains("MISSING_KEY"), "{error}");
}

#[test]
fn treats_an_empty_openai_key_environment_variable_as_unset() {
    let settings =
        load("litellm_settings: {}\n", &[("OPENAI_API_KEY", "")]).expect("settings load");
    assert!(settings.openai_api_key.is_none());
}
