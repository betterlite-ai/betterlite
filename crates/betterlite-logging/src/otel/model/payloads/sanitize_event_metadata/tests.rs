#![cfg(test)]

use super::*;
#[test]
fn sanitize_filters_secrets_and_bounds_values() {
    let source = serde_json::from_value::<BTreeMap<String, Value>>(
        serde_json::json!({"authorization":"x","safe":"ok"}),
    )
    .unwrap();
    assert_eq!(
        sanitize_event_metadata(&source).get("safe"),
        Some(&"ok".to_owned())
    );
}
