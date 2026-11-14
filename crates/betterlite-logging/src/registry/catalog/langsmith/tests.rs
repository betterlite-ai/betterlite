#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_config::loader::ConfigLoader;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::ProxiedClient;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use super::{build, resolve_settings};
use crate::registry::catalog::{CatalogError, CatalogInputs, LoggerCatalog};

fn inputs() -> CatalogInputs {
    let config = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load("")
    .expect("the config loads");
    let policy = Arc::new(
        ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new())
            .expect("the proxy policy resolves"),
    );
    CatalogInputs {
        config: Arc::new(config),
        http: ProxiedClient::outbound(&policy).expect("the HTTP client builds"),
        env: Arc::new(BTreeMap::<String, String>::new()),
        callback_settings: BTreeMap::new(),
        tracker: TaskTracker::new(),
        shutdown: CancellationToken::new(),
    }
}

#[tokio::test]
async fn callback_logger_is_built_once_per_process() {
    let inputs = inputs();
    let tracker = inputs.tracker.clone();
    let catalog = LoggerCatalog::new(inputs);
    assert!(catalog.existing("langsmith").is_none());
    let first = catalog
        .resolve_logger("langsmith")
        .expect("the name is available")
        .expect("the logger builds");
    let second = catalog
        .resolve_logger("langsmith")
        .expect("the name is available")
        .expect("the second resolve reuses the logger");
    assert!(Arc::ptr_eq(&first, &second));
    assert_eq!(first.callback_name(), "langsmith");
    assert_eq!(tracker.len(), 1);
}

#[test]
fn resolve_settings_reads_the_config_batch_size_and_redaction_flag() {
    let config = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load("litellm_settings: {langsmith_batch_size: 10, redact_user_api_key_info: true}")
    .expect("the config loads");
    let mut inputs = inputs();
    inputs.config = Arc::new(config);
    let settings = resolve_settings(&inputs).expect("the settings resolve");
    assert_eq!(settings.batch.batch_size, 10);
    assert!(settings.redact_user_api_key_info);
}

#[tokio::test]
async fn callback_logger_reports_a_non_numeric_batch_size() {
    let mut inputs = inputs();
    inputs.env = Arc::new(BTreeMap::from([(
        "LANGSMITH_BATCH_SIZE".to_owned(),
        "many".to_owned(),
    )]));
    let error = build(&inputs)
        .err()
        .expect("the batch size is not a number");
    assert!(matches!(
        error,
        CatalogError::Init { message } if message.contains("LANGSMITH_BATCH_SIZE")
    ));
}
