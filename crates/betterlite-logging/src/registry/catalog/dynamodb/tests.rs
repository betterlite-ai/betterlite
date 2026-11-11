#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_config::loader::ConfigLoader;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::ProxiedClient;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use crate::registry::catalog::{CatalogError, CatalogInputs};

use super::{build, build_logger};

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

fn configured() -> CatalogInputs {
    inputs(
        "callback_settings:\n  dynamodb:\n    dynamodb_table_name: logs\n",
        &[("AWS_REGION_NAME", "us-east-1")],
    )
}

#[tokio::test]
async fn callback_logger_builds_the_dynamodb_logger() {
    let inputs = configured();
    let logger = build(&inputs).expect("the logger builds");
    assert_eq!(logger.callback_name(), "dynamodb");
}

#[tokio::test]
async fn callback_logger_reads_the_table_and_region_settings() {
    let inputs = configured();
    let logger = build_logger(&inputs).expect("the logger builds");
    assert_eq!(logger.settings.table_name, "logs");
    assert_eq!(logger.settings.region_name, "us-east-1");
    assert_eq!(
        logger.client.endpoint,
        "https://dynamodb.us-east-1.amazonaws.com/"
    );
}

#[tokio::test]
async fn callback_logger_requires_the_table_name() {
    let inputs = inputs("", &[("AWS_REGION_NAME", "us-east-1")]);
    let error = build(&inputs).err().expect("the table is missing");
    assert!(matches!(
        error,
        CatalogError::Init { message } if message.contains("dynamodb_table_name")
    ));
}

#[tokio::test]
async fn callback_logger_requires_the_region_variable() {
    let inputs = inputs(
        "callback_settings:\n  dynamodb:\n    dynamodb_table_name: logs\n",
        &[],
    );
    let error = build(&inputs).err().expect("the region is missing");
    assert!(matches!(
        error,
        CatalogError::Init { message } if message.contains("AWS_REGION_NAME")
    ));
}

#[tokio::test]
async fn callback_logger_refuses_an_unknown_setting() {
    let inputs = inputs(
        "callback_settings:\n  dynamodb:\n    dynamodb_table_name: logs\n    dynamodb_typo: 1\n",
        &[("AWS_REGION_NAME", "us-east-1")],
    );
    let error = build(&inputs).err().expect("the setting is refused");
    assert!(matches!(
        error,
        CatalogError::Init { message } if message.contains("dynamodb_typo")
    ));
}
