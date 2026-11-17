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

const CONFIGURED: &str = "callback_settings:\n  zerobus:\n    server_endpoint: \"https://1234567890123456.zerobus.us-west-2.cloud.databricks.com\"\n    workspace_url: \"https://dbc-1a2b3c4d.cloud.databricks.com\"\n    client_id: sp-client-id\n    client_secret: sp-client-secret\n    table_name: main.litellm.traces\n";

fn inputs(source: &str, variables: &[(&str, &str)]) -> CatalogInputs {
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
    let environment: BTreeMap<String, String> = variables
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

async fn shutdown(inputs: &CatalogInputs) {
    inputs.shutdown.cancel();
    inputs.tracker.close();
    timeout(Duration::from_secs(1), inputs.tracker.wait())
        .await
        .expect("the periodic flush stops on shutdown");
}

#[tokio::test]
async fn callback_logger_builds_the_zerobus_logger() {
    let inputs = inputs(CONFIGURED, &[]);
    let logger = build(&inputs).expect("the logger builds");
    assert_eq!(logger.callback_name(), "zerobus");
    assert_eq!(inputs.tracker.len(), 1);
    shutdown(&inputs).await;
}

#[tokio::test]
async fn callback_logger_reads_the_environment_settings() {
    let inputs = inputs(
        "callback_settings:\n  zerobus:\n    table_name: main.litellm.traces\n",
        &[
            (
                "ZEROBUS_SERVER_ENDPOINT",
                "https://1234567890123456.zerobus.us-west-2.cloud.databricks.com",
            ),
            (
                "ZEROBUS_WORKSPACE_URL",
                "https://dbc-1a2b3c4d.cloud.databricks.com",
            ),
            ("ZEROBUS_CLIENT_ID", "sp-client-id"),
            ("ZEROBUS_CLIENT_SECRET", "sp-client-secret"),
        ],
    );
    let logger = build(&inputs).expect("the environment completes the connection");
    assert_eq!(logger.callback_name(), "zerobus");
    shutdown(&inputs).await;
}

#[tokio::test]
async fn callback_logger_reports_the_message_logging_setting() {
    let configured = format!("{CONFIGURED}    turn_off_message_logging: true\n");
    let inputs = inputs(&configured, &[]);
    let logger = build(&inputs).expect("the logger builds");
    assert!(logger.turn_off_message_logging());
    shutdown(&inputs).await;
}

#[tokio::test]
async fn callback_logger_requires_the_connection_settings() {
    let inputs = inputs("", &[]);
    let error = build(&inputs).err().expect("the connection is missing");
    assert!(matches!(
        error,
        CatalogError::Init { message } if message.contains("ZEROBUS_SERVER_ENDPOINT")
    ));
}

#[tokio::test]
async fn callback_logger_refuses_an_unknown_setting() {
    let inputs = inputs(
        "callback_settings:\n  zerobus:\n    bogus_setting: 1\n",
        &[],
    );
    let error = build(&inputs).err().expect("the setting is unknown");
    assert!(matches!(
        error,
        CatalogError::Init { message } if message.contains("bogus_setting")
    ));
}

#[tokio::test]
async fn callback_logger_refuses_a_table_that_is_not_fully_qualified() {
    let configured = CONFIGURED.replace("main.litellm.traces", "main.traces");
    let inputs = inputs(&configured, &[]);
    let error = build(&inputs).err().expect("the table is not qualified");
    assert!(matches!(
        error,
        CatalogError::Init { message } if message.contains("main.traces")
    ));
}

#[tokio::test]
async fn callback_logger_refuses_a_non_numeric_workspace_id() {
    let configured = CONFIGURED.replace(
        "https://1234567890123456.zerobus.us-west-2.cloud.databricks.com",
        "https://dbc-1a2b3c4d.zerobus.us-west-2.cloud.databricks.com",
    );
    let inputs = inputs(&configured, &[]);
    let error = build(&inputs)
        .err()
        .expect("the workspace id is not numeric");
    assert!(matches!(
        error,
        CatalogError::Init { message } if message.contains("dbc-1a2b3c4d")
    ));
}
