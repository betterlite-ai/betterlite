#![cfg(test)]

use super::ActivityAvailability;

#[test]
fn defaults_both_flags_to_false() {
    let decoded: ActivityAvailability = serde_json::from_str("{}").expect("decodes");
    assert_eq!(
        decoded,
        ActivityAvailability {
            traces: false,
            requests: false,
        }
    );
}

#[test]
fn decodes_both_flags_set() {
    let decoded: ActivityAvailability =
        serde_json::from_str(r#"{"traces": true, "requests": true}"#).expect("decodes");
    assert_eq!(
        decoded,
        ActivityAvailability {
            traces: true,
            requests: true,
        }
    );
}

#[test]
fn rejects_unknown_fields() {
    let error = serde_json::from_str::<ActivityAvailability>(r#"{"traces": true, "extra": 1}"#);
    error.unwrap_err();
}

#[test]
fn rejects_non_boolean_traces() {
    let error = serde_json::from_str::<ActivityAvailability>(r#"{"traces": "yes"}"#);
    error.unwrap_err();
}
