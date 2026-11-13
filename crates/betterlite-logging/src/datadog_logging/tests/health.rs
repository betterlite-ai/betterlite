use betterlite_models::decisions::Json;

use crate::datadog_logging::settings::DatadogInit;
use crate::datadog_logging::tests::support;
use crate::datadog_logging::{health, inspection};
use crate::exporter_options::health::HealthState;

fn direct() -> DatadogInit {
    support::direct_init("key-1", "us5.datadoghq.com")
}

fn message_of(call: &support::Call) -> Json {
    let entries = call.entries();
    let first = entries.first().expect("the health check sends one payload");
    let Json::Object(envelope) = first else {
        panic!("a health payload is an object");
    };
    let Some(Json::Text(message)) = envelope.get("message") else {
        panic!("a health payload carries a message");
    };
    serde_json::from_str(message).expect("the message is JSON")
}

#[tokio::test]
async fn datadog_health_check_behavior() {
    let (logger, transport) = support::build(
        &direct(),
        &support::Environment::new(),
        vec![support::Reply::accepted()],
    );
    let healthy = health::async_health_check(&logger).await;
    assert_eq!(healthy.status, HealthState::Healthy);
    assert_eq!(healthy.error_message, None);
    assert_eq!(transport.call_count(), 1);

    let (failing, _transport) = support::build(
        &direct(),
        &support::Environment::new(),
        vec![support::Reply::body(400, "bad request")],
    );
    let unhealthy = health::async_health_check(&failing).await;
    assert_eq!(unhealthy.status, HealthState::Unhealthy);
    assert_eq!(unhealthy.error_message.as_deref(), Some("bad request"));
}

#[tokio::test]
async fn datadog_health_check_reports_a_non_202_status() {
    let (logger, _transport) = support::build(
        &direct(),
        &support::Environment::new(),
        vec![support::Reply::body(200, "not accepted")],
    );
    let unhealthy = health::async_health_check(&logger).await;
    assert_eq!(unhealthy.status, HealthState::Unhealthy);
    assert_eq!(unhealthy.error_message.as_deref(), Some("not accepted"));
}

#[tokio::test]
async fn datadog_health_check_reports_a_transport_error() {
    let (logger, _transport) = support::build(
        &direct(),
        &support::Environment::new(),
        vec![support::Reply::network()],
    );
    let unhealthy = health::async_health_check(&logger).await;
    assert_eq!(unhealthy.status, HealthState::Unhealthy);
    assert!(
        unhealthy
            .error_message
            .is_some_and(|message| !message.is_empty())
    );
}

#[tokio::test]
async fn datadog_health_check_sends_the_dummy_payload() {
    let (logger, transport) = support::build(
        &direct(),
        &support::Environment::new(),
        vec![support::Reply::accepted()],
    );
    let _status = health::async_health_check(&logger).await;
    let calls = transport.calls();
    assert_eq!(calls[0].header("content-encoding"), Some("gzip"));
    assert_eq!(calls[0].header("dd-api-key"), Some("key-1"));
    let message = message_of(&calls[0]);
    let Json::Object(fields) = &message else {
        panic!("the dummy message is an object");
    };
    assert_eq!(fields.get("id"), Some(&Json::Text("test_id".to_owned())));
    assert_eq!(
        fields.get("model"),
        Some(&Json::Text("gpt-3.5-turbo".to_owned()))
    );
    assert_eq!(
        fields.get("total_tokens"),
        Some(&Json::Number(serde_json::Number::from(30_u64)))
    );
    let entries = calls[0].entries();
    let Json::Object(envelope) = &entries[0] else {
        panic!("a health payload is an object");
    };
    assert_eq!(envelope.get("status"), Some(&Json::Text("info".to_owned())));
}

#[tokio::test]
async fn datadog_payload_inspection_noop_behavior() {
    let (logger, _transport) = support::build(&direct(), &support::Environment::new(), Vec::new());
    assert!(inspection::get_request_response_payload(&logger, "request-id", None, None).is_none());
}
