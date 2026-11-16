#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;
use secrecy::ExposeSecret as _;

use crate::loader::{ConfigError, ConfigLoader};
use crate::types::Config;

fn load(source: &str) -> Result<Config, ConfigError> {
    ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load(source)
}

#[test]
fn loads_bucket_service_account_and_path() {
    let config = load("litellm_settings:\n  cache: true\n  cache_params:\n    type: gcs\n    gcs_bucket_name: responses\n    gcs_path_service_account: /run/sa.json\n    gcs_path: responses//\n")
        .expect("valid GCS settings");
    let settings = config.sections.gcs_response_cache.unwrap();
    assert_eq!(settings.bucket_name, "responses");
    assert_eq!(
        settings.service_account_path.unwrap().expose_secret(),
        "/run/sa.json"
    );
    assert_eq!(settings.path, "responses//");
}

#[test]
fn missing_path_is_empty() {
    let config = load("litellm_settings:\n  cache: true\n  cache_params:\n    type: gcs\n    gcs_bucket_name: responses\n")
        .expect("valid GCS settings");
    let settings = config.sections.gcs_response_cache.unwrap();
    assert_eq!(settings.path, "");
}

#[test]
fn missing_service_account_is_none() {
    let config = load("litellm_settings:\n  cache: true\n  cache_params:\n    type: gcs\n    gcs_bucket_name: responses\n")
        .expect("valid GCS settings");
    let settings = config.sections.gcs_response_cache.unwrap();
    assert!(settings.service_account_path.is_none());
}

#[test]
fn falls_back_to_bucket_and_service_account_environment() {
    let mut lookup = BTreeMap::new();
    lookup.insert("GCS_BUCKET_NAME".to_owned(), "responses".to_owned());
    lookup.insert(
        "GCS_PATH_SERVICE_ACCOUNT".to_owned(),
        "/run/sa.json".to_owned(),
    );
    let config = ConfigLoader { lookup }
        .load("litellm_settings:\n  cache: true\n  cache_params:\n    type: gcs\n")
        .expect("environment-backed GCS settings");
    let settings = config.sections.gcs_response_cache.unwrap();
    assert_eq!(settings.bucket_name, "responses");
    assert_eq!(
        settings.service_account_path.unwrap().expose_secret(),
        "/run/sa.json"
    );
}

#[test]
fn explicit_values_beat_environment() {
    let mut lookup = BTreeMap::new();
    lookup.insert("GCS_BUCKET_NAME".to_owned(), "from-environment".to_owned());
    lookup.insert(
        "GCS_PATH_SERVICE_ACCOUNT".to_owned(),
        "/environment/sa.json".to_owned(),
    );
    let config = ConfigLoader { lookup }
        .load("litellm_settings:\n  cache: true\n  cache_params:\n    type: gcs\n    gcs_bucket_name: from-config\n    gcs_path_service_account: /config/sa.json\n")
        .expect("configured GCS settings");
    let settings = config.sections.gcs_response_cache.unwrap();
    assert_eq!(settings.bucket_name, "from-config");
    assert_eq!(
        settings.service_account_path.unwrap().expose_secret(),
        "/config/sa.json"
    );
}

#[test]
fn empty_values_fall_back_to_environment() {
    let mut lookup = BTreeMap::new();
    lookup.insert("GCS_BUCKET_NAME".to_owned(), "from-environment".to_owned());
    lookup.insert(
        "GCS_PATH_SERVICE_ACCOUNT".to_owned(),
        "/environment/sa.json".to_owned(),
    );
    let config = ConfigLoader { lookup }
        .load("litellm_settings:\n  cache: true\n  cache_params:\n    type: gcs\n    gcs_bucket_name: ''\n    gcs_path_service_account: ''\n")
        .expect("environment-backed GCS settings");
    let settings = config.sections.gcs_response_cache.unwrap();
    assert_eq!(settings.bucket_name, "from-environment");
    assert_eq!(
        settings.service_account_path.unwrap().expose_secret(),
        "/environment/sa.json"
    );
}

#[test]
fn empty_environment_values_are_unset() {
    let mut lookup = BTreeMap::new();
    lookup.insert("GCS_BUCKET_NAME".to_owned(), String::new());
    lookup.insert("GCS_PATH_SERVICE_ACCOUNT".to_owned(), String::new());
    let error = ConfigLoader { lookup }
        .load("litellm_settings:\n  cache: true\n  cache_params:\n    type: gcs\n")
        .expect_err("the bucket is missing");
    assert!(matches!(error, ConfigError::MissingEnvironment { .. }));
}

#[test]
fn missing_bucket_is_an_error() {
    let error = load("litellm_settings:\n  cache: true\n  cache_params:\n    type: gcs\n")
        .expect_err("the bucket is required");
    assert!(matches!(
        error,
        ConfigError::MissingEnvironment {
            variable,
            path,
        } if variable == "GCS_BUCKET_NAME"
            && path == "litellm_settings.cache_params.gcs_bucket_name"
    ));
}

#[test]
fn resolves_environment_references() {
    let mut lookup = BTreeMap::new();
    lookup.insert("BUCKET".to_owned(), "responses".to_owned());
    lookup.insert("CREDENTIALS".to_owned(), "/run/sa.json".to_owned());
    lookup.insert("PREFIX".to_owned(), "cache".to_owned());
    let config = ConfigLoader { lookup }
        .load("litellm_settings:\n  cache: true\n  cache_params:\n    type: gcs\n    gcs_bucket_name: os.environ/BUCKET\n    gcs_path_service_account: os.environ/CREDENTIALS\n    gcs_path: os.environ/PREFIX\n")
        .expect("environment references resolve");
    let settings = config.sections.gcs_response_cache.unwrap();
    assert_eq!(settings.bucket_name, "responses");
    assert_eq!(
        settings.service_account_path.unwrap().expose_secret(),
        "/run/sa.json"
    );
    assert_eq!(settings.path, "cache");
}

#[rstest]
#[case::bucket(
    "gcs_bucket_name: os.environ/UNSET",
    "litellm_settings.cache_params.gcs_bucket_name"
)]
#[case::service_account(
    "gcs_bucket_name: responses\n    gcs_path_service_account: os.environ/UNSET",
    "litellm_settings.cache_params.gcs_path_service_account"
)]
#[case::path(
    "gcs_bucket_name: responses\n    gcs_path: os.environ/UNSET",
    "litellm_settings.cache_params.gcs_path"
)]
fn missing_reference_names_its_path(#[case] params: &str, #[case] path: &str) {
    let yaml =
        format!("litellm_settings:\n  cache: true\n  cache_params:\n    type: gcs\n    {params}\n");
    let error = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load(&yaml)
    .expect_err("the reference is unresolved");
    assert!(matches!(
        error,
        ConfigError::MissingEnvironment {
            variable,
            path: error_path,
        } if variable == "UNSET" && error_path == path
    ));
}

#[test]
fn empty_service_account_environment_is_none() {
    let mut lookup = BTreeMap::new();
    lookup.insert("GCS_PATH_SERVICE_ACCOUNT".to_owned(), String::new());
    let config = ConfigLoader { lookup }
        .load("litellm_settings:\n  cache: true\n  cache_params:\n    type: gcs\n    gcs_bucket_name: responses\n")
        .expect("the bucket comes from the config");
    let settings = config.sections.gcs_response_cache.unwrap();
    assert!(settings.service_account_path.is_none());
}

#[test]
fn cache_false_yields_none() {
    let config = load("litellm_settings:\n  cache: false\n  cache_params:\n    type: gcs\n    gcs_bucket_name: responses\n")
        .expect("cache is off");
    assert!(config.sections.gcs_response_cache.is_none());
}

#[test]
fn missing_cache_params_yields_none() {
    let config = load("litellm_settings:\n  cache: true\n").expect("cache without params");
    assert!(config.sections.gcs_response_cache.is_none());
}

#[test]
fn other_cache_type_yields_none() {
    let config = load("litellm_settings:\n  cache: true\n  cache_params:\n    type: redis\n    gcs_bucket_name: responses\n")
        .expect("another backend");
    assert!(config.sections.gcs_response_cache.is_none());
}

#[test]
fn missing_cache_type_yields_none() {
    let config =
        load("litellm_settings:\n  cache: true\n  cache_params:\n    gcs_bucket_name: responses\n")
            .expect("no backend type");
    assert!(config.sections.gcs_response_cache.is_none());
}

#[test]
fn absent_section_is_none() {
    let config = load("").expect("an empty config");
    assert!(config.sections.gcs_response_cache.is_none());
}
