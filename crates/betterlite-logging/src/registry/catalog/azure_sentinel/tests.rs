#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_config::loader::ConfigLoader;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::ProxiedClient;
use rstest::rstest;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use super::build;
use crate::registry::catalog::{CatalogError, CatalogInputs, LoggerCatalog};

const REQUIRED: [(&str, &str); 5] = [
    ("AZURE_SENTINEL_DCR_IMMUTABLE_ID", "dcr-1"),
    ("AZURE_SENTINEL_ENDPOINT", "https://dce.example.test"),
    ("AZURE_SENTINEL_TENANT_ID", "tenant-1"),
    ("AZURE_SENTINEL_CLIENT_ID", "client-1"),
    ("AZURE_SENTINEL_CLIENT_SECRET", "secret-1"),
];

fn inputs(variables: &[(&str, &str)]) -> CatalogInputs {
    let policy = Arc::new(
        ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).expect("valid proxy policy"),
    );
    let environment: BTreeMap<String, String> = variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    CatalogInputs {
        config: Arc::new(
            ConfigLoader {
                lookup: BTreeMap::new(),
            }
            .load("")
            .expect("valid config"),
        ),
        http: ProxiedClient::outbound(&policy).expect("http client"),
        env: Arc::new(environment),
        callback_settings: BTreeMap::new(),
        tracker: TaskTracker::new(),
        shutdown: CancellationToken::new(),
    }
}

#[tokio::test]
async fn the_builder_makes_a_logger_named_azure_sentinel_from_the_environment() {
    let inputs = inputs(&REQUIRED);
    let logger = build(&inputs).expect("the required environment builds a logger");
    assert_eq!(logger.callback_name(), "azure_sentinel");
    inputs.shutdown.cancel();
    inputs.tracker.close();
    inputs.tracker.wait().await;
}

#[tokio::test]
async fn the_periodic_flush_runs_on_the_servers_task_tracker() {
    let inputs = inputs(&REQUIRED);
    let _logger = build(&inputs).expect("the required environment builds a logger");
    assert_eq!(inputs.tracker.len(), 1);
    inputs.shutdown.cancel();
    inputs.tracker.close();
    inputs.tracker.wait().await;
    assert!(inputs.tracker.is_empty());
}

#[rstest]
#[case::dcr(
    "AZURE_SENTINEL_DCR_IMMUTABLE_ID",
    "AZURE_SENTINEL_DCR_IMMUTABLE_ID is required. Set it as an environment variable or pass dcr_immutable_id parameter."
)]
#[case::endpoint(
    "AZURE_SENTINEL_ENDPOINT",
    "AZURE_SENTINEL_ENDPOINT is required. Set it as an environment variable or pass endpoint parameter."
)]
#[case::client_secret(
    "AZURE_SENTINEL_CLIENT_SECRET",
    "AZURE_SENTINEL_CLIENT_SECRET or AZURE_CLIENT_SECRET is required. Set it as an environment variable or pass client_secret parameter."
)]
#[tokio::test]
async fn a_missing_setting_is_an_init_error_with_litellms_message(
    #[case] missing: &str,
    #[case] message: &str,
) {
    let present: Vec<(&str, &str)> = REQUIRED
        .iter()
        .copied()
        .filter(|(name, _value)| *name != missing)
        .collect();
    let inputs = inputs(&present);
    let error = build(&inputs)
        .err()
        .expect("a missing setting fails the build");
    assert!(matches!(error, CatalogError::Init { .. }));
    assert_eq!(error.to_string(), message);
    assert!(inputs.tracker.is_empty());
}

#[tokio::test]
async fn the_catalog_hands_out_the_logger_by_its_callback_name() {
    let catalog = LoggerCatalog::new(inputs(&REQUIRED));
    let logger = catalog
        .resolve_logger("azure_sentinel")
        .expect("the name is available")
        .expect("the required environment builds a logger");
    assert_eq!(logger.callback_name(), "azure_sentinel");
    let again = catalog
        .resolve_logger("azure_sentinel")
        .expect("the name is available")
        .expect("the logger is reused");
    assert!(Arc::ptr_eq(&logger, &again));
}

#[tokio::test]
async fn the_catalog_skips_the_logger_when_a_setting_is_missing() {
    let catalog = LoggerCatalog::new(inputs(&[]));
    assert!(
        catalog
            .resolve_logger("azure_sentinel")
            .expect("an init error is not a resolve error")
            .is_none()
    );
}
