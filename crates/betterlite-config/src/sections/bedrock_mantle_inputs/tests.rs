#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;
use secrecy::{ExposeSecret, SecretString};

use crate::loader::{ConfigError, ConfigLoader};

use super::Settings;

#[derive(Debug, Clone, Copy)]
enum Field {
    ApiBase,
    ApiKey,
    Region,
    Unrelated,
}

fn load(environment: &[(&str, &str)]) -> Settings {
    let lookup = environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect::<BTreeMap<_, _>>();
    ConfigLoader { lookup }
        .load("{}")
        .unwrap()
        .sections
        .bedrock_mantle_inputs
}

fn exposed(secret: Option<&SecretString>) -> Option<&str> {
    secret.map(ExposeSecret::expose_secret)
}

#[rstest]
#[case::loads_bedrock_mantle_api_base(Field::ApiBase)]
#[case::loads_bedrock_mantle_api_key(Field::ApiKey)]
#[case::loads_bedrock_mantle_region(Field::Region)]
#[case::rejects_unrelated_key(Field::Unrelated)]
fn bedrock_mantle_inputs_config_cases(#[case] field: Field) {
    match field {
        Field::ApiBase => loads_api_base(),
        Field::ApiKey => loads_api_key(),
        Field::Region => loads_region(),
        Field::Unrelated => rejects_unrelated_keys(),
    }
}

fn loads_api_base() {
    let default = load(&[]);
    assert!(default.api_base.is_none());
    assert!(default.api_key.is_none());
    assert!(default.region.is_none());
    let configured = load(&[("BEDROCK_MANTLE_API_BASE", "https://mantle.example/v1")]);
    assert_eq!(
        configured.api_base.as_deref(),
        Some("https://mantle.example/v1")
    );
    assert!(configured.api_key.is_none());
    assert!(configured.region.is_none());
    // the resolver applies LiteLLM's truthiness, so an empty value is kept as given
    let empty = load(&[("BEDROCK_MANTLE_API_BASE", "")]);
    assert_eq!(empty.api_base.as_deref(), Some(""));
}

fn loads_api_key() {
    let default = load(&[]);
    assert!(default.api_key.is_none());
    assert!(default.api_base.is_none());
    assert!(default.region.is_none());
    let configured = load(&[("BEDROCK_MANTLE_API_KEY", "fixture-mantle-key")]);
    assert_eq!(
        exposed(configured.api_key.as_ref()),
        Some("fixture-mantle-key")
    );
    assert!(configured.api_base.is_none());
    assert!(configured.region.is_none());
    let empty = load(&[("BEDROCK_MANTLE_API_KEY", "")]);
    assert_eq!(exposed(empty.api_key.as_ref()), Some(""));
    assert!(!format!("{empty:?}").contains("BEDROCK_MANTLE_API_KEY"));
}

fn loads_region() {
    let default = load(&[]);
    assert!(default.region.is_none());
    assert!(default.api_base.is_none());
    assert!(default.api_key.is_none());
    let configured = load(&[("BEDROCK_MANTLE_REGION", "eu-west-1")]);
    assert_eq!(configured.region.as_deref(), Some("eu-west-1"));
    assert!(configured.api_base.is_none());
    assert!(configured.api_key.is_none());
    let empty = load(&[("BEDROCK_MANTLE_REGION", "")]);
    assert_eq!(empty.region.as_deref(), Some(""));
}

fn rejects_unrelated_keys() {
    let lookup = BTreeMap::new();
    let error = ConfigLoader { lookup }
        .load("litellm_settings:\n  bedrock_mantle_api_base: https://mantle.example/v1\n")
        .unwrap_err();
    assert!(
        matches!(
            &error,
            ConfigError::Unsupported { paths }
                if *paths == ["litellm_settings.bedrock_mantle_api_base"]
        ),
        "{error}"
    );
    let nested_lookup = BTreeMap::new();
    let nested_error = ConfigLoader {
        lookup: nested_lookup,
    }
    .load("bedrock_mantle_inputs:\n  api_key: fixture\n")
    .unwrap_err();
    assert!(
        matches!(
            &nested_error,
            ConfigError::Unsupported { paths }
                if *paths == ["bedrock_mantle_inputs"]
        ),
        "{nested_error}"
    );
}
