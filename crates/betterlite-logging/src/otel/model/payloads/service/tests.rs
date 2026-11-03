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
}
