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
use crate::sqs_logging::tests::support::{event, payload};

use super::build;

const KEY_B64: &str = "BwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwc=";

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

fn configured() -> CatalogInputs {
    inputs(format!(
        "callback_settings:\n  aws_sqs:\n    sqs_queue_url: https://queue.example.com\n    sqs_region_name: us-east-1\n    sqs_aws_access_key_id: access\n    sqs_aws_secret_access_key: secret\n    sqs_aws_use_application_level_encryption: true\n    sqs_app_encryption_key_b64: {KEY_B64}\n"
    )
    .as_str())
}

#[tokio::test]
async fn callback_logger_builds_the_aws_sqs_logger() {
    let inputs = configured();
    let logger = build(&inputs).expect("the logger builds");
    assert_eq!(logger.callback_name(), "aws_sqs");
    assert_eq!(inputs.tracker.len(), 1);
    logger
        .async_log_success_event(event(payload()))
        .await
        .expect("the event is recorded");
}

#[tokio::test]
async fn callback_logger_reports_a_missing_encryption_key() {
    let inputs = inputs(
        "callback_settings:\n  aws_sqs:\n    sqs_aws_use_application_level_encryption: true\n",
    );
    let error = build(&inputs).err().expect("the key is missing");
    assert!(matches!(
        error,
        CatalogError::Init { message } if message.contains("sqs_app_encryption_key_b64")
    ));
}

#[tokio::test]
async fn callback_logger_refuses_an_unknown_setting() {
    let inputs = inputs("callback_settings:\n  aws_sqs:\n    sqs_bogus_setting: 1\n");
    let error = build(&inputs).err().expect("the setting is unknown");
    assert!(matches!(
        error,
        CatalogError::Init { message } if message.contains("sqs_bogus_setting")
    ));
}

#[tokio::test(start_paused = true)]
async fn callback_logger_flush_loop_stops_on_shutdown() {
    let inputs = configured();
    let _logger = build(&inputs).expect("the logger builds");
    inputs.shutdown.cancel();
    inputs.tracker.close();
    let awaited = timeout(Duration::from_secs(1), inputs.tracker.wait()).await;
    awaited.unwrap();
}
