#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_config::loader::{ConfigLoader, EnvironmentLookup};
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::ProxiedClient;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use crate::callback_logger::event::LoggerEvent;
use crate::fields::payload::StandardLoggingPayload;
use crate::registry::catalog::{CatalogError, CatalogInputs};
use crate::sqs_logging::health::HEALTH_CHECK_PAYLOAD;

use super::build;

const LOCAL_API_URL: &str = "http://127.0.0.1:9";

fn environment(pairs: &[(&str, &str)]) -> Arc<dyn EnvironmentLookup + Send + Sync> {
    let variables: BTreeMap<String, String> = pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    Arc::new(variables)
}

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
    CatalogInputs {
        config: Arc::new(config),
        http: ProxiedClient::outbound(&policy).expect("the HTTP client builds"),
        env: environment(variables),
        callback_settings,
        tracker: TaskTracker::new(),
        shutdown: CancellationToken::new(),
    }
}

fn configured(extra: &str) -> CatalogInputs {
    inputs(
        format!("litellm_settings:\n  pointfive_params:\n    api_url: {LOCAL_API_URL}\n{extra}")
            .as_str(),
        &[],
    )
}

fn event() -> Arc<LoggerEvent> {
    let payload: StandardLoggingPayload =
        serde_json::from_str(HEALTH_CHECK_PAYLOAD).expect("the dummy payload parses");
    Arc::new(LoggerEvent {
        payload: Arc::new(payload),
        dynamic_params: Arc::new(Json::Object(IndexMap::new())),
        litellm_metadata: Arc::new(IndexMap::new()),
        stream: false,
        guardrail_snapshot: None,
    })
}

#[tokio::test]
async fn callback_logger_builds_from_the_configured_key() {
    let inputs = configured("    api_key: configured-key\n");
    let logger = build(&inputs).expect("the logger builds");
    assert_eq!(logger.callback_name(), "pointfive");
    assert_eq!(inputs.tracker.len(), 1);
}

#[tokio::test]
async fn callback_logger_builds_from_the_environment_key() {
    let inputs = inputs(
        format!("litellm_settings:\n  pointfive_params:\n    api_url: {LOCAL_API_URL}\n").as_str(),
        &[("POINTFIVE_API_KEY", "environment-key")],
    );
    let logger = build(&inputs).expect("the logger builds");
    assert_eq!(logger.callback_name(), "pointfive");
}

#[tokio::test]
async fn callback_logger_refuses_a_missing_key() {
    let inputs = configured("");
    let error = build(&inputs).err().expect("the key is missing");
    assert!(
        matches!(
            &error,
            CatalogError::Init { message }
                if message.contains("pointfive logging requires an api key")
        ),
        "{error}"
    );
    assert_eq!(inputs.tracker.len(), 0);
}

#[tokio::test]
async fn callback_logger_reads_the_configured_redaction_flag() {
    let inputs = configured("    api_key: configured-key\n    turn_off_message_logging: true\n");
    let logger = build(&inputs).expect("the logger builds");
    assert!(logger.turn_off_message_logging());
}

#[tokio::test]
async fn callback_logger_buffers_the_events_it_is_given() {
    let inputs = configured("    api_key: configured-key\n");
    let logger = build(&inputs).expect("the logger builds");
    logger
        .async_log_success_event(event())
        .await
        .expect("the event is queued");
    logger
        .async_log_failure_event(event())
        .await
        .expect("the event is queued");
}
