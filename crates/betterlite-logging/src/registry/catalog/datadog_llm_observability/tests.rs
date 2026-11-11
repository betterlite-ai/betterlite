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

fn inputs(source: &str, environment: BTreeMap<String, String>) -> CatalogInputs {
    let policy = Arc::new(
        ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).expect("valid proxy policy"),
    );
    let config = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load(source)
    .expect("valid config");
    let callback_settings = config.sections.callback_settings.callback_settings.clone();
    CatalogInputs {
        config: Arc::new(config),
        http: ProxiedClient::outbound(&policy).expect("http client"),
        env: Arc::new(environment),
        callback_settings,
        tracker: TaskTracker::new(),
        shutdown: CancellationToken::new(),
    }
}

fn empty() -> CatalogInputs {
    inputs("", BTreeMap::new())
}

#[tokio::test]
async fn callback_logger_builds_in_agent_mode() {
    let environment =
        BTreeMap::from([("LITELLM_DD_AGENT_HOST".to_owned(), "agent.local".to_owned())]);
    let logger = build(&inputs("", environment)).expect("the logger builds in agent mode");
    assert_eq!(logger.callback_name(), "datadog_llm_observability");
}

#[tokio::test]
async fn callback_logger_builds_in_direct_api_mode() {
    let environment = BTreeMap::from([
        ("DD_API_KEY".to_owned(), "secret-key".to_owned()),
        ("DD_SITE".to_owned(), "datadoghq.eu".to_owned()),
    ]);
    let logger = build(&inputs("", environment)).expect("the logger builds with credentials");
    assert_eq!(logger.callback_name(), "datadog_llm_observability");
}

#[tokio::test]
async fn callback_logger_reports_missing_credentials() {
    let Err(error) = build(&empty()) else {
        panic!("a build without credentials must fail");
    };
    assert!(matches!(error, CatalogError::Init { .. }));
    assert!(error.to_string().contains("DD_API_KEY"));
}

#[tokio::test]
async fn callback_logger_reads_turn_off_message_logging() {
    let source =
        "callback_settings:\n  datadog_llm_observability:\n    turn_off_message_logging: true\n";
    let environment =
        BTreeMap::from([("LITELLM_DD_AGENT_HOST".to_owned(), "agent.local".to_owned())]);
    let logger = build(&inputs(source, environment)).expect("the logger builds");
    assert!(logger.turn_off_message_logging());
}

#[tokio::test]
async fn callback_logger_reports_an_unknown_callback_setting() {
    let source = "callback_settings:\n  datadog_llm_observability:\n    not_a_field: true\n";
    let environment =
        BTreeMap::from([("LITELLM_DD_AGENT_HOST".to_owned(), "agent.local".to_owned())]);
    let Err(error) = build(&inputs(source, environment)) else {
        panic!("an unknown setting must fail the build");
    };
    assert!(matches!(error, CatalogError::Init { .. }));
    assert!(error.to_string().contains("not_a_field"));
}

#[tokio::test]
async fn callback_logger_is_cached_by_the_catalog() {
    let environment =
        BTreeMap::from([("LITELLM_DD_AGENT_HOST".to_owned(), "agent.local".to_owned())]);
    let catalog = LoggerCatalog::new(inputs("", environment));
    let first = catalog
        .resolve_logger("datadog_llm_observability")
        .expect("the callback name is known")
        .expect("the logger builds");
    let second = catalog
        .resolve_logger("datadog_llm_observability")
        .expect("the callback name is known")
        .expect("the logger is cached");
    assert!(Arc::ptr_eq(&first, &second));
}
