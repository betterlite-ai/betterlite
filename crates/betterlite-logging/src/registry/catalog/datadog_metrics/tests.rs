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

fn inputs(environment: BTreeMap<String, String>) -> CatalogInputs {
    let policy = Arc::new(
        ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).expect("valid proxy policy"),
    );
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
async fn callback_logger_builds_the_datadog_metrics_logger() {
    let logger = build(&inputs(BTreeMap::new())).expect("the logger builds without credentials");
    assert_eq!(logger.callback_name(), "datadog_metrics");
}

#[tokio::test]
async fn callback_logger_builds_with_credentials() {
    let environment = BTreeMap::from([
        ("DD_API_KEY".to_owned(), "secret-key".to_owned()),
        ("DD_SITE".to_owned(), "datadoghq.eu".to_owned()),
    ]);
    let logger = build(&inputs(environment)).expect("the logger builds with credentials");
    assert_eq!(logger.callback_name(), "datadog_metrics");
}

#[tokio::test]
async fn callback_logger_reports_an_invalid_batch_size() {
    let environment = BTreeMap::from([("DEFAULT_BATCH_SIZE".to_owned(), "many".to_owned())]);
    let Err(error) = build(&inputs(environment)) else {
        panic!("an invalid DEFAULT_BATCH_SIZE must fail the build");
    };
    assert!(matches!(error, CatalogError::Init { .. }));
    assert!(error.to_string().contains("DEFAULT_BATCH_SIZE"));
}

#[tokio::test]
async fn callback_logger_is_cached_by_the_catalog() {
    let environment = BTreeMap::from([("DD_API_KEY".to_owned(), "secret-key".to_owned())]);
    let catalog = LoggerCatalog::new(inputs(environment));
    let first = catalog
        .resolve_logger("datadog_metrics")
        .expect("the callback name is known")
        .expect("the logger builds");
    let second = catalog
        .resolve_logger("datadog_metrics")
        .expect("the callback name is known")
        .expect("the logger is cached");
    assert!(Arc::ptr_eq(&first, &second));
}
