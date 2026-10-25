#![cfg(test)]

use super::*;

#[test]
fn bedrock_records_round_trip_camel_case_fields() {
    let value: Response = serde_json::from_str(r#"{"usage":{"futureCounter":3},"action":"NONE"}"#)
        .expect("valid record");
    assert_eq!(value.action.as_deref(), Some("NONE"));
    assert!(value.usage.expect("usage").0.contains_key("futureCounter"));
}
