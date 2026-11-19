#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_config::loader::{ConfigLoader, EnvironmentLookup};
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::ProxiedClient;
use tokio::time::timeout;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use crate::cold_storage_key::{ColdStorageLocator as _, ColdStorageTarget};
use crate::cold_storage_locator::CatalogColdStorage;
use crate::registry::catalog::{CatalogInputs, LoggerCatalog};
use crate::s3_transport::options::PartitionGranularity;

#[derive(Debug, Default)]
struct MutableEnvironment(Mutex<BTreeMap<String, String>>);

impl MutableEnvironment {
    fn set(&self, name: &str, value: &str) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(name.to_owned(), value.to_owned());
    }

    fn unset(&self, name: &str) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(name);
    }
}

impl EnvironmentLookup for MutableEnvironment {
    fn lookup(&self, name: &str) -> Option<String> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(name)
            .cloned()
    }
}

fn environment() -> Arc<MutableEnvironment> {
    let environment = Arc::new(MutableEnvironment::default());
    environment.set("AWS_ACCESS_KEY_ID", "AKIDTEST");
    environment.set("AWS_SECRET_ACCESS_KEY", "secrettest");
    environment
}

fn inputs(source_text: &str, environment: Arc<MutableEnvironment>) -> CatalogInputs {
    let config = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load(source_text)
    .expect("the config loads");
    let callback_settings = config.sections.callback_settings.callback_settings.clone();
    let policy = Arc::new(
        ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new())
            .expect("the proxy policy resolves"),
    );
    CatalogInputs {
        config: Arc::new(config),
        http: ProxiedClient::outbound(&policy).expect("the HTTP client builds"),
        env: environment,
        callback_settings,
        tracker: TaskTracker::new(),
        shutdown: CancellationToken::new(),
    }
}

#[tokio::test]
async fn a_missing_name_locates_nothing() {
    let catalog = Arc::new(LoggerCatalog::new(inputs("", environment())));
    let locator = CatalogColdStorage::new(None, catalog);
    assert_eq!(locator.locate(), None);
}

#[tokio::test]
async fn a_name_with_no_built_logger_keeps_the_default_target() {
    let catalog = Arc::new(LoggerCatalog::new(inputs("", environment())));
    let s3_v2 = CatalogColdStorage::new(Some("s3_v2".to_owned()), Arc::clone(&catalog));
    assert_eq!(s3_v2.locate(), Some(ColdStorageTarget::default()));
    let unknown = CatalogColdStorage::new(Some("not_a_logger".to_owned()), Arc::clone(&catalog));
    assert_eq!(unknown.locate(), Some(ColdStorageTarget::default()));
}

#[tokio::test]
async fn another_logger_keeps_the_default_target() {
    let inputs = inputs(
        "callback_settings:\n  s3:\n    s3_bucket_name: logs\n    s3_region_name: us-east-1\n",
        environment(),
    );
    let tracker = inputs.tracker.clone();
    let shutdown = inputs.shutdown.clone();
    let catalog = Arc::new(LoggerCatalog::new(inputs));
    let built = catalog
        .resolve_logger("s3")
        .expect("the s3 logger resolves")
        .expect("the s3 logger builds");
    assert_eq!(built.cold_storage_target(), None);
    let locator = CatalogColdStorage::new(Some("s3".to_owned()), Arc::clone(&catalog));
    assert_eq!(locator.locate(), Some(ColdStorageTarget::default()));
    shutdown.cancel();
    tracker.close();
    timeout(Duration::from_secs(1), tracker.wait())
        .await
        .expect("the built logger stops");
}

#[tokio::test]
async fn a_built_s3_v2_logger_reports_its_path_and_current_granularity() {
    let environment = environment();
    environment.set("S3_PARTITION_GRANULARITY", "hour");
    let inputs = inputs(
        "callback_settings:\n  s3_v2:\n    s3_bucket_name: logs\n    s3_region_name: us-east-1\n    s3_endpoint_url: http://127.0.0.1:9000\n    s3_path: team\n",
        Arc::clone(&environment),
    );
    let tracker = inputs.tracker.clone();
    let shutdown = inputs.shutdown.clone();
    let catalog = Arc::new(LoggerCatalog::new(inputs));
    let built = catalog
        .resolve_logger("s3_v2")
        .expect("the s3_v2 logger resolves");
    assert!(built.is_some());
    let locator = CatalogColdStorage::new(Some("s3_v2".to_owned()), Arc::clone(&catalog));
    assert_eq!(
        locator.locate(),
        Some(ColdStorageTarget {
            s3_path: "team".to_owned(),
            partition_granularity: PartitionGranularity::Hour,
        })
    );
    environment.unset("S3_PARTITION_GRANULARITY");
    assert_eq!(
        locator.locate(),
        Some(ColdStorageTarget {
            s3_path: "team".to_owned(),
            partition_granularity: PartitionGranularity::Day,
        })
    );
    shutdown.cancel();
    tracker.close();
    timeout(Duration::from_secs(1), tracker.wait())
        .await
        .expect("the flush loop stops on shutdown");
}
