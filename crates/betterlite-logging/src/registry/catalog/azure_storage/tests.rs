#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_config::loader::ConfigLoader;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::ProxiedClient;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use super::build;
use crate::registry::catalog::{CatalogError, CatalogInputs, LoggerCatalog};

const REQUIRED: [(&str, &str); 2] = [
    ("AZURE_STORAGE_ACCOUNT_NAME", "acct"),
    ("AZURE_STORAGE_FILE_SYSTEM", "logs"),
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
async fn callback_logger_builds_with_an_account_key() {
    let variables = [
        ("AZURE_STORAGE_ACCOUNT_NAME", "acct"),
        ("AZURE_STORAGE_FILE_SYSTEM", "logs"),
        ("AZURE_STORAGE_ACCOUNT_KEY", "a2V5"),
    ];
    let inputs = inputs(&variables);
    let logger = build(&inputs).expect("the required environment builds a logger");
    assert_eq!(logger.callback_name(), "azure_storage");
    inputs.shutdown.cancel();
    inputs.tracker.close();
    inputs.tracker.wait().await;
}

#[tokio::test]
async fn callback_logger_builds_with_entra_credentials() {
    let variables = [
        ("AZURE_STORAGE_ACCOUNT_NAME", "acct"),
        ("AZURE_STORAGE_FILE_SYSTEM", "logs"),
        ("AZURE_STORAGE_TENANT_ID", "tenant-1"),
        ("AZURE_STORAGE_CLIENT_ID", "client-1"),
        ("AZURE_STORAGE_CLIENT_SECRET", "secret-1"),
    ];
    let inputs = inputs(&variables);
    let logger = build(&inputs).expect("the service principal builds a logger");
    assert_eq!(logger.callback_name(), "azure_storage");
    inputs.shutdown.cancel();
    inputs.tracker.close();
    inputs.tracker.wait().await;
}

#[tokio::test]
async fn callback_logger_reports_a_missing_required_variable() {
    let inputs = inputs(&REQUIRED[..1]);
    let error = build(&inputs).err().expect("the account name is missing");
    assert!(matches!(error, CatalogError::Init { .. }));
    assert!(error.to_string().contains("AZURE_STORAGE_FILE_SYSTEM"));
}

#[tokio::test]
async fn callback_logger_resolves_through_the_catalog() {
    let inputs = inputs(&REQUIRED);
    let catalog = LoggerCatalog::new(inputs);
    let logger = catalog
        .resolve_logger("azure_storage")
        .expect("the adapter is installed")
        .expect("the environment builds the logger");
    assert_eq!(logger.callback_name(), "azure_storage");
}
