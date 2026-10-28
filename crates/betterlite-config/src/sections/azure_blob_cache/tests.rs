#![cfg(test)]

use std::collections::BTreeMap;

use crate::loader::{ConfigError, ConfigLoader};

#[test]
fn loads_required_account_url_and_container() {
    let config = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load("litellm_settings:\n  cache: true\n  cache_params:\n    type: azure-blob\n    azure_account_url: https://example.blob.core.windows.net\n    azure_blob_container: responses\n")
    .expect("valid Azure Blob settings");
    let settings = config.sections.azure_blob_cache.unwrap();
    assert_eq!(
        settings.account_url,
        "https://example.blob.core.windows.net"
    );
    assert_eq!(settings.container, "responses");
}

#[test]
fn resolves_values_from_environment() {
    let mut lookup = BTreeMap::new();
    lookup.insert(
        "ACCOUNT_URL".to_owned(),
        "https://example.blob.core.windows.net".to_owned(),
    );
    lookup.insert("CONTAINER".to_owned(), "responses".to_owned());
    let config = ConfigLoader { lookup }
        .load("litellm_settings:\n  cache: true\n  cache_params:\n    type: azure-blob\n    azure_account_url: os.environ/ACCOUNT_URL\n    azure_blob_container: os.environ/CONTAINER\n")
        .expect("environment-backed settings");
    let settings = config.sections.azure_blob_cache.unwrap();
    assert_eq!(
        settings.account_url,
        "https://example.blob.core.windows.net"
    );
    assert_eq!(settings.container, "responses");
}

#[test]
fn rejects_missing_account_url() {
    let error = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load("litellm_settings:\n  cache: true\n  cache_params:\n    type: azure-blob\n    azure_blob_container: responses\n")
    .expect_err("account url is required");
    assert!(matches!(error, ConfigError::MissingEnvironment { .. }));
}

#[test]
fn rejects_missing_container() {
    let error = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load("litellm_settings:\n  cache: true\n  cache_params:\n    type: azure-blob\n    azure_account_url: https://example.blob.core.windows.net\n")
    .expect_err("container is required");
    assert!(matches!(error, ConfigError::MissingEnvironment { .. }));
}

#[test]
fn absent_section_is_none() {
    let config = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load("")
    .unwrap();
    assert!(config.sections.azure_blob_cache.is_none());
}

#[test]
fn cache_false_yields_none() {
    let config = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load("litellm_settings:\n  cache: false\n  cache_params:\n    type: azure-blob\n    azure_account_url: https://example.blob.core.windows.net\n    azure_blob_container: responses\n")
    .unwrap();
    assert!(config.sections.azure_blob_cache.is_none());
}

#[test]
fn other_cache_type_yields_none() {
    let config = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load("litellm_settings:\n  cache: true\n  cache_params:\n    type: redis\n    azure_account_url: https://example.blob.core.windows.net\n    azure_blob_container: responses\n")
    .unwrap();
    assert!(config.sections.azure_blob_cache.is_none());
}

#[test]
fn missing_cache_type_yields_none() {
    let config = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load("litellm_settings:\n  cache: true\n  cache_params:\n    azure_account_url: https://example.blob.core.windows.net\n    azure_blob_container: responses\n")
    .unwrap();
    assert!(config.sections.azure_blob_cache.is_none());
}

#[test]
fn s3_type_does_not_load_azure_settings() {
    let config = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load("litellm_settings:\n  cache: true\n  cache_params:\n    type: s3\n    s3_bucket_name: responses\n")
    .unwrap();
    assert!(config.sections.azure_blob_cache.is_none());
}
