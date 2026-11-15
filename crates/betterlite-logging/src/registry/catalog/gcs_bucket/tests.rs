#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use betterlite_config::loader::ConfigLoader;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::ProxiedClient;
use tokio::time::timeout;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use crate::registry::catalog::{CatalogError, CatalogInputs};

use super::build;

fn inputs(environment: &[(&str, &str)]) -> CatalogInputs {
    let config = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load("model_list: []\n")
    .expect("the config loads");
    let callback_settings = config.sections.callback_settings.callback_settings.clone();
    let policy = Arc::new(
        ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new())
            .expect("the proxy policy resolves"),
    );
    let environment: BTreeMap<String, String> = environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    CatalogInputs {
        config: Arc::new(config),
        http: ProxiedClient::outbound(&policy).expect("the HTTP client builds"),
        env: Arc::new(environment),
        callback_settings,
        tracker: TaskTracker::new(),
        shutdown: CancellationToken::new(),
    }
}

#[tokio::test]
async fn callback_logger_builds_the_gcs_bucket_logger() {
    let inputs = inputs(&[("GCS_BUCKET_NAME", "logs")]);
    let logger = build(&inputs).expect("the logger builds");
    assert_eq!(logger.callback_name(), "gcs_bucket");
    assert_eq!(inputs.tracker.len(), 1);
    inputs.shutdown.cancel();
    inputs.tracker.close();
    timeout(Duration::from_secs(1), inputs.tracker.wait())
        .await
        .expect("the flush loop stops on shutdown");
}

#[tokio::test]
async fn callback_logger_builds_without_a_configured_bucket() {
    let inputs = inputs(&[]);
    let logger = build(&inputs).expect("the bucket is resolved at upload time");
    assert_eq!(logger.callback_name(), "gcs_bucket");
    inputs.shutdown.cancel();
    inputs.tracker.close();
    timeout(Duration::from_secs(1), inputs.tracker.wait())
        .await
        .expect("the flush loop stops on shutdown");
}

#[tokio::test]
async fn callback_logger_refuses_an_invalid_batch_size() {
    let inputs = inputs(&[("GCS_BUCKET_NAME", "logs"), ("GCS_BATCH_SIZE", "many")]);
    let error = build(&inputs).err().expect("the batch size is invalid");
    assert!(matches!(
        error,
        CatalogError::Init { message } if message.contains("GCS_BATCH_SIZE")
    ));
    inputs.shutdown.cancel();
    inputs.tracker.close();
    timeout(Duration::from_secs(1), inputs.tracker.wait())
        .await
        .expect("no flush loop is running");
}
