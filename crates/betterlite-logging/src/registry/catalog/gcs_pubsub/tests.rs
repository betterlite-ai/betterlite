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
    .load("")
    .expect("the config loads");
    let policy = Arc::new(
        ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new())
            .expect("the proxy policy resolves"),
    );
    let variables: BTreeMap<String, String> = environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    CatalogInputs {
        config: Arc::new(config),
        http: ProxiedClient::outbound(&policy).expect("the HTTP client builds"),
        env: Arc::new(variables),
        callback_settings: BTreeMap::new(),
        tracker: TaskTracker::new(),
        shutdown: CancellationToken::new(),
    }
}

async fn stop(inputs: &CatalogInputs) {
    inputs.shutdown.cancel();
    inputs.tracker.close();
    timeout(Duration::from_secs(1), inputs.tracker.wait())
        .await
        .expect("the flush loop stops on shutdown");
}

#[tokio::test]
async fn callback_logger_builds_the_gcs_pubsub_logger() {
    let inputs = inputs(&[
        ("GCS_PUBSUB_PROJECT_ID", "project"),
        ("GCS_PUBSUB_TOPIC_ID", "topic"),
    ]);
    let logger = build(&inputs).expect("the logger builds");
    assert_eq!(logger.callback_name(), "gcs_pubsub");
    assert_eq!(inputs.tracker.len(), 1);
    stop(&inputs).await;
}

#[tokio::test]
async fn callback_logger_requires_the_project_and_topic() {
    let missing_project = inputs(&[("GCS_PUBSUB_TOPIC_ID", "topic")]);
    let project_error = build(&missing_project)
        .err()
        .expect("the project is required");
    assert!(matches!(
        project_error,
        CatalogError::Init { message } if message.contains("GCS_PUBSUB_PROJECT_ID")
    ));
    let missing_topic = inputs(&[("GCS_PUBSUB_PROJECT_ID", "project")]);
    let topic_error = build(&missing_topic).err().expect("the topic is required");
    assert!(matches!(
        topic_error,
        CatalogError::Init { message } if message.contains("GCS_PUBSUB_TOPIC_ID")
    ));
}

#[tokio::test]
async fn callback_logger_refuses_an_invalid_batch_size() {
    let inputs = inputs(&[
        ("GCS_PUBSUB_PROJECT_ID", "project"),
        ("GCS_PUBSUB_TOPIC_ID", "topic"),
        ("DEFAULT_BATCH_SIZE", "many"),
    ]);
    let error = build(&inputs).err().expect("the batch size is invalid");
    assert!(matches!(
        error,
        CatalogError::Init { message } if message.contains("DEFAULT_BATCH_SIZE")
    ));
}

#[tokio::test]
async fn callback_logger_accepts_the_use_v1_flag() {
    let inputs = inputs(&[
        ("GCS_PUBSUB_PROJECT_ID", "project"),
        ("GCS_PUBSUB_TOPIC_ID", "topic"),
        ("GCS_PUB_SUB_USE_V1", "true"),
    ]);
    let logger = build(&inputs).expect("the flag is read");
    assert_eq!(logger.callback_name(), "gcs_pubsub");
    stop(&inputs).await;
}

#[tokio::test]
async fn callback_logger_requires_a_boolean_use_v1_flag() {
    let inputs = inputs(&[
        ("GCS_PUBSUB_PROJECT_ID", "project"),
        ("GCS_PUBSUB_TOPIC_ID", "topic"),
        ("GCS_PUB_SUB_USE_V1", "maybe"),
    ]);
    let error = build(&inputs).err().expect("the flag is not a boolean");
    assert!(matches!(
        error,
        CatalogError::Init { message } if message.contains("GCS_PUB_SUB_USE_V1")
    ));
}
