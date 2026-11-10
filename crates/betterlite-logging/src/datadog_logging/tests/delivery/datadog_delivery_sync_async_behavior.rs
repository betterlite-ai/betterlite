use betterlite_models::decisions::Json;

use crate::datadog_logging::payloads::SyncCall;
use crate::datadog_logging::sync;
use crate::datadog_logging::tests::support;

use super::{kwargs, legacy_kwargs, message_field, moment};

#[tokio::test]
async fn v1_fills_the_generated_id() {
    let mut init = support::direct_init("key-1", "us5.datadoghq.com");
    init.use_v1 = true;
    let (logger, transport) = support::build(
        &init,
        &support::Environment::new(),
        vec![support::Reply::accepted()],
    );
    let payload = legacy_kwargs();
    let without_id: Json = serde_json::from_str(r#"{"usage": {}}"#).expect("a response");
    let call = SyncCall {
        kwargs: &payload,
        response_obj: &without_id,
        start_time: moment(),
        end_time: moment(),
    };
    sync::log_success_event(&logger, &call).await;
    assert_eq!(transport.call_count(), 1);
    let record = transport.calls()[0].body.clone();
    assert_eq!(
        message_field(&record, "id"),
        Json::Text("generated-id".to_owned())
    );
    assert_eq!(
        message_field(&record, "call_type"),
        Json::Text("litellm.completion".to_owned())
    );
    assert_eq!(
        message_field(&record, "model"),
        Json::Text("gpt-4o".to_owned())
    );
}

#[tokio::test]
async fn v1_takes_the_response_id() {
    let mut init = support::direct_init("key-1", "us5.datadoghq.com");
    init.use_v1 = true;
    let (logger, transport) = support::build(
        &init,
        &support::Environment::new(),
        vec![support::Reply::accepted()],
    );
    let payload = legacy_kwargs();
    let response: Json =
        serde_json::from_str(r#"{"id": "resp-1", "usage": {}}"#).expect("a response");
    let call = SyncCall {
        kwargs: &payload,
        response_obj: &response,
        start_time: moment(),
        end_time: moment(),
    };
    sync::log_success_event(&logger, &call).await;
    assert_eq!(transport.call_count(), 1);
    assert_eq!(
        message_field(&transport.calls()[0].body, "id"),
        Json::Text("resp-1".to_owned())
    );
}

#[tokio::test]
async fn v1_without_legacy_kwargs_sends_nothing() {
    let mut init = support::direct_init("key-1", "us5.datadoghq.com");
    init.use_v1 = true;
    let (logger, transport) = support::build(
        &init,
        &support::Environment::new(),
        vec![support::Reply::accepted()],
    );
    let payload = kwargs(support::standard_payload("sync-1", false));
    let call = SyncCall {
        kwargs: &payload,
        response_obj: &Json::Null,
        start_time: moment(),
        end_time: moment(),
    };
    sync::log_success_event(&logger, &call).await;
    assert_eq!(transport.call_count(), 0);
}
