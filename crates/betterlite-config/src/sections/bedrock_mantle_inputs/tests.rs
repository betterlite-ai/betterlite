#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;
use secrecy::ExposeSecret;

use crate::loader::{ConfigError, ConfigLoader};

use super::Settings;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Field {
    ApiBase,
    ApiKey,
    Region,
    Unrelated,
}

const FIELDS: [Field; 3] = [Field::ApiBase, Field::ApiKey, Field::Region];

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

fn read(settings: &Settings, field: Field) -> Option<String> {
    let secret = settings.api_key.as_ref().map(ExposeSecret::expose_secret);
    match field {
        Field::ApiBase => settings.api_base.clone(),
        Field::ApiKey => secret.map(str::to_owned),
        Field::Region => settings.region.clone(),
        Field::Unrelated => None,
    }
}

fn set_fields(settings: &Settings) -> Vec<Field> {
    FIELDS
        .into_iter()
        .filter(|field| read(settings, *field).is_some())
        .collect()
}

#[rstest]
#[case::loads_bedrock_mantle_api_base(Field::ApiBase)]
#[case::loads_bedrock_mantle_api_key(Field::ApiKey)]
#[case::loads_bedrock_mantle_region(Field::Region)]
#[case::rejects_unrelated_key(Field::Unrelated)]
fn bedrock_mantle_inputs_config_cases(#[case] field: Field) {
    match field {
        Field::Unrelated => rejects_unrelated_keys(),
        Field::ApiBase | Field::ApiKey | Field::Region => loads_field(field),
    }
}

fn loads_field(field: Field) {
    let variable = match field {
        Field::ApiBase => "BEDROCK_MANTLE_API_BASE",
        Field::ApiKey => "BEDROCK_MANTLE_API_KEY",
        Field::Region => "BEDROCK_MANTLE_REGION",
        Field::Unrelated => return,
    };
    let default = load(&[]);
    assert_eq!(read(&default, field), None);
    assert!(set_fields(&default).is_empty());

    let configured = load(&[(variable, "from-env")]);
    assert_eq!(read(&configured, field).as_deref(), Some("from-env"));
    assert_eq!(set_fields(&configured), [field]);

    // the resolver applies LiteLLM's truthiness, so an empty value is kept as given
    let empty = load(&[(variable, "")]);
    assert_eq!(read(&empty, field).as_deref(), Some(""));
    assert_eq!(set_fields(&empty), [field]);
}

fn rejects_unrelated_keys() {
    let error = ConfigLoader {
        lookup: BTreeMap::new(),
    }
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
    let nested_error = ConfigLoader {
        lookup: BTreeMap::new(),
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

#[test]
fn bedrock_mantle_inputs_keep_the_key_secret() {
    let configured = load(&[("BEDROCK_MANTLE_API_KEY", "fixture-mantle-key")]);
    let rendered = format!("{configured:?}");
    assert!(!rendered.contains("fixture-mantle-key"));
    assert_eq!(
        configured.api_key.as_ref().map(ExposeSecret::expose_secret),
        Some("fixture-mantle-key")
    );
}
