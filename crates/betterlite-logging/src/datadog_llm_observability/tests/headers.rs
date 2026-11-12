use betterlite_models::decisions::Json;

use crate::callback_logger::CallbackLogger as _;
use crate::datadog_llm_observability::flush::queue::flush_queue;
use crate::datadog_llm_observability::settings::DatadogLlmObsInit;
use crate::datadog_llm_observability::tests::support;

async fn deliver(
    init: &DatadogLlmObsInit,
    environment: &support::Environment,
) -> Vec<support::Call> {
    let (logger, transport) = support::build(init, environment, vec![support::Reply::accepted()]);
    logger
        .async_log_success_event(support::event(support::standard_payload("one")))
        .await
        .expect("the event is accepted");
    flush_queue(&logger).await;
    transport.calls()
}

#[tokio::test]
async fn datadog_llm_obs_payload_headers_behavior() {
    let environment = support::environment(&[
        ("DD_API_KEY", "header-key"),
        ("DD_SITE", "us5.datadoghq.com"),
    ]);
    let calls = deliver(&support::init(None, None, None, None), &environment).await;
    assert_eq!(calls.len(), 1);
    assert_eq!(
        calls[0].url,
        "https://api.us5.datadoghq.com/api/intake/llm-obs/v1/trace/spans"
    );
    assert_eq!(calls[0].header("content-type"), Some("application/json"));
    assert_eq!(calls[0].header("dd-api-key"), Some("header-key"));
    assert_eq!(calls[0].header("content-encoding"), None);
}

#[tokio::test]
async fn datadog_llm_obs_payload_body_is_one_uncompressed_span_document() {
    let environment = support::environment(&[("LITELLM_DD_AGENT_HOST", "agent.local")]);
    let calls = deliver(&support::init(None, None, None, None), &environment).await;
    let body = &calls[0].body;
    assert_eq!(
        support::field(body, &["data", "type"]),
        Some(&Json::Text("span".to_owned()))
    );
    assert_eq!(
        support::field(body, &["data", "attributes", "ml_app"]),
        Some(&Json::Text("litellm-server".to_owned()))
    );
    assert_eq!(
        support::field(body, &["data", "attributes", "tags"]),
        Some(&Json::List(vec![
            Json::Text("env:unknown".to_owned()),
            Json::Text("service:litellm-server".to_owned()),
            Json::Text("version:unknown".to_owned()),
            Json::Text("HOSTNAME:".to_owned()),
            Json::Text("POD_NAME:unknown".to_owned()),
        ]))
    );
    let spans = calls[0].spans();
    assert_eq!(spans.len(), 1);
    assert_eq!(
        support::field(&spans[0], &["name"]),
        Some(&Json::Text("litellm_llm_call".to_owned()))
    );
}

#[tokio::test]
async fn datadog_llm_obs_agent_mode_keeps_the_header_a_configured_key_sends() {
    let environment = support::environment(&[
        ("LITELLM_DD_AGENT_HOST", "agent.local"),
        ("DD_API_KEY", "agent-key"),
    ]);
    let calls = deliver(&support::init(None, None, None, None), &environment).await;
    assert_eq!(
        calls[0].url,
        "http://agent.local:8126/api/intake/llm-obs/v1/trace/spans"
    );
    assert_eq!(calls[0].header("dd-api-key"), Some("agent-key"));
}

#[tokio::test]
async fn datadog_llm_obs_payload_headers_are_absent_without_a_key() {
    let environment = support::environment(&[("LITELLM_DD_AGENT_HOST", "agent.local")]);
    let calls = deliver(&support::init(None, None, None, None), &environment).await;
    assert_eq!(calls[0].header("content-type"), Some("application/json"));
    assert_eq!(calls[0].header("dd-api-key"), None);
}
