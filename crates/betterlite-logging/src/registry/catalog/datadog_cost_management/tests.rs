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

use super::build;
use crate::registry::catalog::{CatalogError, CatalogInputs};

fn inputs(source: &str) -> CatalogInputs {
    let config = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load(source)
    .expect("the config loads");
    let callback_settings = config.sections.callback_settings.callback_settings.clone();
    let policy = Arc::new(
        ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new())
            .expect("the proxy policy resolves"),
    );
    CatalogInputs {
        config: Arc::new(config),
        http: ProxiedClient::outbound(&policy).expect("the HTTP client builds"),
        env: Arc::new(BTreeMap::<String, String>::new()),
        callback_settings,
        tracker: TaskTracker::new(),
        shutdown: CancellationToken::new(),
    }
}

#[tokio::test]
async fn callback_logger_builds_the_datadog_cost_management_logger() {
    let inputs = inputs(
        "callback_settings:\n  datadog_cost_management:\n    cost_tag_keys: capability,tier\n    batch_size: 4\n    flush_interval: 2.5\n",
    );
    let logger = build(&inputs).expect("the logger builds");
    assert_eq!(logger.callback_name(), "datadog_cost_management");
    assert_eq!(inputs.tracker.len(), 1);
}

#[tokio::test]
async fn callback_logger_refuses_an_unknown_setting() {
    let inputs = inputs("callback_settings:\n  datadog_cost_management:\n    bogus_setting: 1\n");
    let error = build(&inputs).err().expect("the setting is unknown");
    assert!(matches!(error, CatalogError::Init { message } if message.contains("bogus_setting")));
}

#[tokio::test]
async fn callback_logger_refuses_an_invalid_batch_size() {
    let inputs = inputs("callback_settings:\n  datadog_cost_management:\n    batch_size: many\n");
    let error = build(&inputs).err().expect("the batch size is invalid");
    assert!(matches!(error, CatalogError::Init { message } if message.contains("batch_size")));
}

#[test]
fn callback_logger_rejects_a_cost_tag_keys_yaml_list() {
    let source =
        "callback_settings:\n  datadog_cost_management:\n    cost_tag_keys:\n      - capability\n";
    let error = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load(source)
    .expect_err("a list is not a scalar callback setting");
    assert!(error.to_string().contains("cost_tag_keys"));
}

#[tokio::test(start_paused = true)]
async fn callback_logger_flush_loop_stops_on_shutdown() {
    let inputs = inputs("");
    let _logger = build(&inputs).expect("the logger builds");
    inputs.shutdown.cancel();
    inputs.tracker.close();
    timeout(Duration::from_secs(1), inputs.tracker.wait())
        .await
        .expect("the flush loop ends");
}
