#![cfg(test)]

use super::*;
#[test]
fn service_payload_sanitizes_metadata() {
    let metadata = serde_json::from_value::<BTreeMap<String, Value>>(
        serde_json::json!({"token":"secret","zone":"west"}),
    )
    .unwrap();
    let data = ServiceSpanData::from_payload("embedding".to_owned(), None, None, None, &metadata);
    assert_eq!(data.event_metadata.get("zone"), Some(&"west".to_owned()));
}
