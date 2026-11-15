#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_config::loader::ConfigLoader;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::ProxiedClient;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use super::super::{CatalogInputs, LoggerCatalog};
use crate::callback_logger::event::LoggerEvent;
use crate::langfuse_logger::tests::support as langfuse;
use crate::registry::error::RegistryError;

fn inputs(source: &str, variables: &[(&str, &str)]) -> CatalogInputs {
    let policy = Arc::new(
        ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).expect("valid proxy policy"),
    );
    let lookup: BTreeMap<String, String> = variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    let config = ConfigLoader {
        lookup: lookup.clone(),
    }
    .load(source)
    .expect("valid config");
    let callback_settings = config.sections.callback_settings.callback_settings.clone();
    CatalogInputs {
        config: Arc::new(config),
        http: ProxiedClient::outbound(&policy).expect("http client"),
        env: Arc::new(lookup),
        callback_settings,
        tracker: TaskTracker::new(),
        shutdown: CancellationToken::new(),
    }
}

fn event() -> LoggerEvent {
    LoggerEvent {
        payload: Arc::new(langfuse::payload(langfuse::TRACE_ID)),
        dynamic_params: Arc::new(Json::Object(IndexMap::new())),
        litellm_metadata: Arc::new(IndexMap::new()),
        stream: false,
        guardrail_snapshot: None,
    }
}

const CONFIGURED: &str = "litellm_settings:\n  success_callback: [langfuse]\n";

const CALLBACK_SETTINGS_ONLY: &str =
    "callback_settings:\n  langfuse:\n    langfuse_secret: sk-secret\n";

#[tokio::test]
async fn callback_logger_resolves_when_the_config_lists_langfuse() {
    let catalog = LoggerCatalog::new(inputs(CONFIGURED, &[("LANGFUSE_MOCK", "true")]));
    let logger = catalog
        .resolve_logger("langfuse")
        .expect("the name is available")
        .expect("the logger builds");
    assert_eq!(logger.callback_name(), "langfuse");
}

#[tokio::test]
async fn callback_logger_is_unavailable_without_a_config_entry() {
    let catalog = LoggerCatalog::new(inputs("", &[("LANGFUSE_MOCK", "true")]));
    assert!(matches!(
        catalog.resolve_logger("langfuse"),
        Err(RegistryError::UnavailableAdapter { name }) if name == "langfuse"
    ));
}

// set_callbacks only builds a callback named in one of the three lists, so parameters alone enable nothing
#[tokio::test]
async fn callback_logger_is_unavailable_when_only_callback_settings_lists_it() {
    let catalog = LoggerCatalog::new(inputs(CALLBACK_SETTINGS_ONLY, &[("LANGFUSE_MOCK", "true")]));
    assert!(matches!(
        catalog.resolve_logger("langfuse"),
        Err(RegistryError::UnavailableAdapter { name }) if name == "langfuse"
    ));
}

#[tokio::test]
async fn callback_logger_takes_success_and_failure_events() {
    let catalog = LoggerCatalog::new(inputs(CONFIGURED, &[("LANGFUSE_MOCK", "true")]));
    let logger = catalog
        .resolve_logger("langfuse")
        .expect("the name is available")
        .expect("the logger builds");
    logger
        .log_success_event(&event())
        .expect("the success event is accepted");
    logger
        .log_failure_event(&event())
        .expect("the failure event is accepted");
    assert_eq!(
        catalog
            .resolve_logger("langfuse")
            .expect("the name stays available")
            .expect("the built logger is reused")
            .callback_name(),
        "langfuse"
    );
}
