#![cfg(test)]

use std::collections::BTreeMap;

use crate::loader::{ConfigError, ConfigLoader};

fn major_version(environment: Option<&str>) -> u32 {
    let lookup = environment
        .map(|value| ("DEFAULT_REDIS_MAJOR_VERSION".to_owned(), value.to_owned()))
        .into_iter()
        .collect::<BTreeMap<_, _>>();
    ConfigLoader { lookup }
        .load("{}")
        .expect("config should load")
        .sections
        .redis_cache_primitives
        .default_redis_major_version
}

#[test]
fn defaults_to_seven_without_an_environment_override() {
    assert_eq!(major_version(None), 7);
}

#[test]
fn reads_the_major_version_from_the_environment() {
    assert_eq!(major_version(Some("6")), 6);
}

#[test]
fn trims_surrounding_whitespace() {
    assert_eq!(major_version(Some("  8  ")), 8);
}

#[test]
fn rejects_a_non_numeric_override() {
    let lookup = BTreeMap::from([(
        "DEFAULT_REDIS_MAJOR_VERSION".to_owned(),
        "latest".to_owned(),
    )]);
    let error = ConfigLoader { lookup }.load("{}").unwrap_err();
    assert!(matches!(
        error,
        ConfigError::InvalidInteger { path, .. } if path == "DEFAULT_REDIS_MAJOR_VERSION"
    ));
}

#[test]
fn rejects_top_level_yaml_key() {
    let error = ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load("DEFAULT_REDIS_MAJOR_VERSION: 6")
    .unwrap_err();
    assert!(matches!(
        error,
        ConfigError::Unsupported { paths } if paths == ["DEFAULT_REDIS_MAJOR_VERSION"]
    ));
}
