#![cfg(test)]

use super::*;
#[test]
fn service_payload_sanitizes_metadata() {
    let metadata = serde_json::from_str::<BTreeMap<String, OtelModelValue>>(
        r#"{"token":"secret","zone":"west"}"#,
    )
    .unwrap();
    let payload = ServiceLoggerPayload {
        service: ServiceType::Redis,
        call_type: "get".to_owned(),
        caller: None,
        error: None,
    };
    let data = ServiceSpanData::from_payload(&payload, &metadata);
    assert_eq!(data.event_metadata.get("zone"), Some(&"west".to_owned()));
    assert!(!data.event_metadata.contains_key("token"));
    assert_eq!(data.service_name, "redis");
    assert_eq!(data.call_type.as_deref(), Some("get"));
    assert!(data.error.is_none());
}

#[test]
fn service_carries_caller_and_failure() {
    let payload = ServiceLoggerPayload {
        service: ServiceType::RedisDailyEndUserQueue,
        call_type: "write".to_owned(),
        caller: Some("worker".to_owned()),
        error: Some("failed".to_owned()),
    };
    let data = ServiceSpanData::from_payload(&payload, &BTreeMap::new());
    assert_eq!(data.service_name, "redis_daily_end_user_spend_update_queue");
    assert_eq!(data.caller.as_deref(), Some("worker"));
    assert_eq!(data.error.unwrap().message.as_deref(), Some("failed"));
}
