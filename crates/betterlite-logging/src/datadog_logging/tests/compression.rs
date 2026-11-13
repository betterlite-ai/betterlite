use betterlite_models::decisions::Json;
use rstest::rstest;

use crate::callback_logger::CallbackLogger as _;
use crate::datadog_logging::batch;
use crate::datadog_logging::tests::support;

use super::support::message_id;

#[tokio::test]
async fn datadog_compression_headers_behavior() {
    let (logger, transport) = support::build(
        &support::direct_init("key-1", "us5.datadoghq.com"),
        &support::Environment::new(),
        vec![support::Reply::accepted()],
    );
    logger
        .async_log_success_event(support::event(support::standard_payload("one", false)))
        .await
        .expect("the async event is accepted");
    batch::flush_queue(&logger).await;
    let calls = transport.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].header("content-encoding"), Some("gzip"));
    assert_eq!(calls[0].header("content-type"), Some("application/json"));
    assert_eq!(calls[0].header("dd-api-key"), Some("key-1"));
    assert_eq!(
        calls[0].url,
        "https://http-intake.logs.us5.datadoghq.com/api/v2/logs"
    );
    let entries = calls[0].entries();
    assert_eq!(entries.len(), 1);
    assert_eq!(message_id(&entries[0]).as_deref(), Some("one"));
    let Json::Object(envelope) = &entries[0] else {
        panic!("an entry is an object");
    };
    assert_eq!(envelope.get("status"), Some(&Json::Text("info".to_owned())));
    assert_eq!(
        envelope.get("ddsource"),
        Some(&Json::Text("litellm".to_owned()))
    );
}

#[tokio::test]
async fn datadog_compression_omits_the_api_key_without_one() {
    let (logger, transport) = support::build(
        &support::agent_init(),
        &support::Environment::new(),
        vec![support::Reply::accepted()],
    );
    logger
        .async_log_success_event(support::event(support::standard_payload("one", false)))
        .await
        .expect("the async event is accepted");
    batch::flush_queue(&logger).await;
    let calls = transport.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].header("dd-api-key"), None);
    assert_eq!(calls[0].header("content-encoding"), Some("gzip"));
    assert_eq!(calls[0].url, "http://agent.local:10518/api/v2/logs");
}

#[rstest]
#[case::environment_key_allowed(true, Some("env-key"))]
#[case::environment_key_hidden(false, None)]
#[tokio::test]
async fn datadog_agent_credentials_resolution_behavior(
    #[case] allow_env_credentials: bool,
    #[case] expected: Option<&str>,
) {
    let init = support::init_with(None, None, Some("agent.local"), None, allow_env_credentials);
    let (logger, transport) = support::build(
        &init,
        &support::environment(&[("DD_API_KEY", "env-key")]),
        vec![support::Reply::accepted()],
    );
    logger
        .async_log_success_event(support::event(support::standard_payload("one", false)))
        .await
        .expect("the async event is accepted");
    batch::flush_queue(&logger).await;
    let calls = transport.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].header("dd-api-key"), expected);
    assert_eq!(logger.settings.api_key.is_some(), expected.is_some());
}
