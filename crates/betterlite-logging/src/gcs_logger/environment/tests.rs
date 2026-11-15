#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::gcs_client::gcs_storage_auth::environment_source::EnvironmentSource as _;
use crate::gcs_client::gcs_storage_auth::{BUCKET_NAME_ENV, SERVICE_ACCOUNT_ENV};
use crate::gcs_mock::settings::{ENABLE_ENV, LATENCY_ENV};

use super::CatalogEnvironment;

fn catalog_environment(pairs: &[(&str, &str)]) -> CatalogEnvironment {
    let lookup: BTreeMap<String, String> = pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    CatalogEnvironment {
        lookup: Arc::new(lookup),
    }
}

#[test]
fn the_lookup_reads_the_named_setting() {
    let environment = catalog_environment(&[(BUCKET_NAME_ENV, "logs")]);
    assert_eq!(environment.get(BUCKET_NAME_ENV).as_deref(), Some("logs"));
    assert_eq!(environment.get("GCS_OTHER"), None);
}

#[test]
fn the_snapshot_holds_the_bucket_credentials_and_mock_settings() {
    let environment = catalog_environment(&[
        (BUCKET_NAME_ENV, "logs"),
        (SERVICE_ACCOUNT_ENV, "/creds/service.json"),
        (ENABLE_ENV, "true"),
        (LATENCY_ENV, "25"),
        ("GCS_BATCH_SIZE", "10"),
    ]);
    let expected: BTreeMap<String, String> = [
        (BUCKET_NAME_ENV, "logs"),
        (SERVICE_ACCOUNT_ENV, "/creds/service.json"),
        (ENABLE_ENV, "true"),
        (LATENCY_ENV, "25"),
    ]
    .into_iter()
    .map(|(name, value)| (name.to_owned(), value.to_owned()))
    .collect();
    assert_eq!(environment.snapshot(), expected);
}

#[test]
fn the_snapshot_leaves_out_settings_the_environment_lacks() {
    let environment = catalog_environment(&[(BUCKET_NAME_ENV, "logs")]);
    let snapshot = environment.snapshot();
    assert_eq!(snapshot.len(), 1);
    assert_eq!(
        snapshot.get(BUCKET_NAME_ENV).map(String::as_str),
        Some("logs")
    );
}
