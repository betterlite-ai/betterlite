#![cfg(test)]

use super::*;
#[test]
fn sanitize_filters_secrets_and_bounds_values() {
    let source = serde_json::from_str::<BTreeMap<String, OtelModelValue>>(
        r#"{"authorization":"x","safe":"ok","function_duration":12,"function_name":"drop"}"#,
    )
    .unwrap();
    assert_eq!(
        sanitize_event_metadata(&source).get("safe"),
        Some(&"ok".to_owned())
    );
    assert_eq!(
        sanitize_event_metadata(&source).get("function_duration"),
        Some(&"12".to_owned())
    );
}
