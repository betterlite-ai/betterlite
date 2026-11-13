#![cfg(test)]

use super::Detection;

#[test]
fn wire_detection_defaults_missing_fields() {
    let detection: Detection = serde_json::from_str(r#"{"text": "hello"}"#).expect("detection");
    assert_eq!(detection.score.to_bits(), 0.0_f64.to_bits());
    assert_eq!(detection.detection_type, "unknown");
    assert_eq!(detection.text, "hello");
    assert_eq!(detection.detector_id, None);
}

#[test]
fn wire_detection_keeps_present_fields() {
    let detection: Detection = serde_json::from_str(
        r#"{"detection_type":"toxic","score":0.912,"text":"bad","detector_id":"email"}"#,
    )
    .expect("detection");
    assert_eq!(detection.score.to_bits(), 0.912_f64.to_bits());
    assert_eq!(detection.detection_type, "toxic");
    assert_eq!(detection.text, "bad");
    assert_eq!(detection.detector_id.as_deref(), Some("email"));
}

#[test]
fn wire_detection_serializes_the_resolved_defaults() {
    let detection = Detection::default();
    assert_eq!(
        serde_json::to_string(&detection).expect("json"),
        r#"{"text":"","detection_type":"unknown","score":0.0}"#
    );
}
