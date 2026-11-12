#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use super::{AZURE_DEFAULT_API_VERSION, DEFAULT_MAX_RETRIES, Settings};
use crate::loader::{ConfigError, ConfigLoader};

fn load_source(source: &str, variables: &[(&str, &str)]) -> Result<Settings, ConfigError> {
    let lookup: BTreeMap<String, String> = variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    ConfigLoader { lookup }
        .load(source)
        .map(|config| config.sections.azure_endpoints)
}

fn load(source: &str, variables: &[(&str, &str)]) -> Settings {
    load_source(source, variables).unwrap()
}

#[rstest]
#[case::loads_default_max_retries(0)]
#[case::loads_azure_default_api_version(1)]
#[case::rejects_unrelated_key(2)]
fn azure_clients_config_cases(#[case] case: u8) {
    match case {
        0 => loads_default_max_retries(),
        1 => loads_azure_default_api_version(),
        _ => rejects_unrelated_key(),
    }
}

fn loads_default_max_retries() {
    retries_from_environment();
    retries_from_config();
}

fn retries_from_environment() {
    assert_eq!(DEFAULT_MAX_RETRIES, 2);
    assert_eq!(load("", &[]).default_max_retries, DEFAULT_MAX_RETRIES);
    assert_eq!(
        load("", &[("DEFAULT_MAX_RETRIES", "5")]).default_max_retries,
        5
    );
    assert_eq!(
        load("", &[("DEFAULT_MAX_RETRIES", "")]).default_max_retries,
        DEFAULT_MAX_RETRIES
    );
    assert_ne!(
        load("", &[("DEFAULT_MAX_RETRIES", "-1")]).default_max_retries,
        DEFAULT_MAX_RETRIES
    );
    assert!(matches!(
        load_source("", &[("DEFAULT_MAX_RETRIES", "many")]),
        Err(ConfigError::Environment { .. })
    ));
}

fn retries_from_config() {
    let global = load(
        "litellm_settings:\n  DEFAULT_MAX_RETRIES: 7\n",
        &[("DEFAULT_MAX_RETRIES", "5")],
    );
    assert_eq!(global.default_max_retries, 7);
    let referenced = load(
        "litellm_settings:\n  DEFAULT_MAX_RETRIES: os.environ/AZURE_CLIENTS_RETRIES\n",
        &[("AZURE_CLIENTS_RETRIES", "3")],
    );
    assert_eq!(referenced.default_max_retries, 3);
    assert_eq!(
        load("litellm_settings:\n  DEFAULT_MAX_RETRIES: 0\n", &[]).default_max_retries,
        0
    );
    assert!(matches!(
        load_source("litellm_settings:\n  DEFAULT_MAX_RETRIES: many\n", &[]),
        Err(ConfigError::InvalidInteger { .. })
    ));
    let seed = load(
        include_str!("../../../fuzz/corpus/config_yaml/azure_clients-default-max-retries.yaml"),
        &[
            ("AZURE_CLIENTS_RETRIES", "6"),
            ("AZURE_CLIENTS_VERSION", "2024-06-01"),
        ],
    );
    assert_eq!(seed.default_max_retries, 6);
    assert_eq!(seed.azure_default_api_version, "2024-06-01");
}

fn loads_azure_default_api_version() {
    assert_eq!(AZURE_DEFAULT_API_VERSION, "2025-02-01-preview");
    assert_eq!(
        load("", &[]).azure_default_api_version,
        AZURE_DEFAULT_API_VERSION
    );
    assert_eq!(
        load("", &[("AZURE_DEFAULT_API_VERSION", "2024-10-21")]).azure_default_api_version,
        "2024-10-21"
    );
    assert_eq!(
        load("", &[("AZURE_DEFAULT_API_VERSION", "")]).azure_default_api_version,
        AZURE_DEFAULT_API_VERSION
    );
    let global = load(
        "litellm_settings:\n  AZURE_DEFAULT_API_VERSION: 2024-02-01\n",
        &[("AZURE_DEFAULT_API_VERSION", "2024-10-21")],
    );
    assert_eq!(global.azure_default_api_version, "2024-02-01");
    let referenced = load(
        "litellm_settings:\n  AZURE_DEFAULT_API_VERSION: os.environ/AZURE_CLIENTS_VERSION\n",
        &[("AZURE_CLIENTS_VERSION", "2024-08-01-preview")],
    );
    assert_eq!(referenced.azure_default_api_version, "2024-08-01-preview");
    let empty = load(
        "litellm_settings:\n  AZURE_DEFAULT_API_VERSION: \"\"\n",
        &[("AZURE_DEFAULT_API_VERSION", "2024-10-21")],
    );
    assert_eq!(empty.azure_default_api_version, "");
    let seed = load(
        include_str!("../../../fuzz/corpus/config_yaml/azure_clients-defaults.yaml"),
        &[],
    );
    assert_eq!(seed, Settings::default());
}

fn rejects_unrelated_key() {
    let error = load_source("litellm_settings:\n  DEFAULT_MAX_RETRIES_TYPO: 2\n", &[]).unwrap_err();
    let ConfigError::Unsupported { paths } = error else {
        panic!("the unrelated key loaded without an unsupported-key error");
    };
    assert_eq!(
        paths,
        vec!["litellm_settings.DEFAULT_MAX_RETRIES_TYPO".to_owned()]
    );
}
