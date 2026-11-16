#![cfg(test)]

use std::collections::BTreeMap;

use secrecy::ExposeSecret as _;

use super::Settings;
use crate::loader::{ConfigError, ConfigLoader};

const CONFIG: &str =
    "model_list:\n  - model_name: gpt-4o\n    litellm_params:\n      model: openai/gpt-4o\n";

fn load(source: &str, environment: &[(&str, &str)]) -> Result<Settings, ConfigError> {
    let lookup = environment
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect::<BTreeMap<_, _>>();
    Ok(ConfigLoader { lookup }
        .load(source)?
        .sections
        .xecguard_runtime)
}

fn loaded(environment: &[(&str, &str)]) -> Settings {
    load(CONFIG, environment).expect("a loadable config")
}

#[test]
fn xecguard_runtime_leaves_unset_variables_empty() {
    let settings = loaded(&[]);
    assert!(settings.api_key.is_none());
    assert!(settings.api_base.is_none());
    assert!(settings.block_on_error.is_none());
}

#[test]
fn xecguard_runtime_reads_the_api_key() {
    let settings = loaded(&[("XECGUARD_API_KEY", "key-from-environment")]);
    let key = settings.api_key.expect("the api key");
    assert_eq!(key.expose_secret(), "key-from-environment");
}

#[test]
fn xecguard_runtime_reads_the_api_base() {
    let settings = loaded(&[("XECGUARD_API_BASE", "http://xecguard.test:8080/")]);
    assert_eq!(
        settings.api_base.as_deref(),
        Some("http://xecguard.test:8080/")
    );
}

#[test]
fn xecguard_runtime_reads_the_block_on_error_text() {
    let settings = loaded(&[("XECGUARD_BLOCK_ON_ERROR", "YES")]);
    assert_eq!(settings.block_on_error.as_deref(), Some("YES"));
}

#[test]
fn xecguard_runtime_treats_an_empty_variable_as_unset() {
    let settings = loaded(&[
        ("XECGUARD_API_KEY", ""),
        ("XECGUARD_API_BASE", ""),
        ("XECGUARD_BLOCK_ON_ERROR", ""),
    ]);
    assert!(settings.api_key.is_none());
    assert!(settings.api_base.is_none());
    assert!(settings.block_on_error.is_none());
}

#[test]
fn xecguard_runtime_debug_never_prints_the_api_key() {
    let settings = loaded(&[("XECGUARD_API_KEY", "key-that-must-not-be-printed")]);
    let printed = format!("{settings:?}");
    assert!(printed.contains("api_key"));
    assert!(!printed.contains("key-that-must-not-be-printed"));
}

#[test]
fn xecguard_runtime_reads_no_config_key() {
    let source = "xecguard_runtime:\n  api_key: key-from-config\n";
    assert!(matches!(
        load(source, &[]),
        Err(ConfigError::Unsupported { .. })
    ));
}
