#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_config::tls::Verification;
use secrecy::SecretString;

use super::{detector_server_message, orchestrator_message};
use crate::ibm_detector_client::config::IbmDetectorConfig;
use crate::ibm_detector_client::wire::Detection;

fn config(score_threshold: Option<f64>) -> IbmDetectorConfig {
    IbmDetectorConfig {
        guardrail_name: "ibm".to_owned(),
        auth_token: SecretString::from("token"),
        base_url: "https://detector.test".to_owned(),
        api_url: "https://detector.test/api/v1/text/contents".to_owned(),
        detector_id: "jailbreak".to_owned(),
        is_detector_server: true,
        detector_params: BTreeMap::new(),
        extra_headers: BTreeMap::new(),
        score_threshold,
        block_on_detection: true,
        ssl_verification: Verification::Enabled,
        timeout: None,
    }
}

fn detection(detection_type: &str, score: f64, text: &str, detector_id: Option<&str>) -> Detection {
    Detection {
        start: None,
        end: None,
        text: text.to_owned(),
        detection: None,
        detection_type: detection_type.to_owned(),
        score,
        evidences: None,
        metadata: None,
        detector_id: detector_id.map(ToOwned::to_owned),
    }
}

const DETECTOR_SERVER_HEADER: &str =
    "IBM Guardrail Detector failed: 1 violation(s) detected\n\nIBM Guardrail Detector failed:\n\n";

#[test]
fn message_detector_server_lists_each_message() {
    let config = config(None);
    let detections = vec![vec![detection("toxic", 0.9_f64, "bad", None)], Vec::new()];
    assert_eq!(
        detector_server_message(&config, &detections),
        format!("{DETECTOR_SERVER_HEADER}Message 1:\n  - TOXIC (score: 0.900)\n    Text: 'bad'")
    );
}

#[test]
fn message_detector_server_keeps_message_numbers_for_skipped_messages() {
    let config = config(None);
    let detections = vec![Vec::new(), vec![detection("pii", 0.5_f64, "email", None)]];
    assert_eq!(
        detector_server_message(&config, &detections),
        format!("{DETECTOR_SERVER_HEADER}Message 2:\n  - PII (score: 0.500)\n    Text: 'email'")
    );
}

#[test]
fn message_detector_server_applies_the_threshold() {
    let config = config(Some(0.8_f64));
    let detections = vec![vec![
        detection("toxic", 0.5_f64, "mild", None),
        detection("toxic", 0.9_f64, "bad", None),
    ]];
    assert_eq!(
        detector_server_message(&config, &detections),
        format!("{DETECTOR_SERVER_HEADER}Message 1:\n  - TOXIC (score: 0.900)\n    Text: 'bad'")
    );
}

#[test]
fn message_detector_server_names_an_unknown_detection_type() {
    let config = config(None);
    let detections = vec![vec![detection("unknown", 0.0_f64, "", None)]];
    assert_eq!(
        detector_server_message(&config, &detections),
        "IBM Guardrail Detector failed: 1 violation(s) detected\n\nIBM Guardrail Detector failed:\n\nMessage 1:\n  - UNKNOWN (score: 0.000)\n    Text: ''"
    );
}

#[test]
fn message_detector_server_without_violations_reports_zero() {
    let config = config(None);
    assert_eq!(
        detector_server_message(&config, &[Vec::new(), Vec::new()]),
        "IBM Guardrail Detector failed: 0 violation(s) detected\n\nIBM Guardrail Detector failed:"
    );
}

#[test]
fn message_orchestrator_lists_each_detection_with_its_detector() {
    let config = config(None);
    let detections = vec![
        detection("pii", 0.85_f64, "email", Some("email-detector")),
        detection("toxic", 1.0_f64, "bad", None),
    ];
    assert_eq!(
        orchestrator_message(&config, &detections),
        "IBM Guardrail Detector failed: 2 violation(s) detected\n\n- PII (detector: email-detector, score: 0.850)\n  Text: 'email'\n\n- TOXIC (detector: jailbreak, score: 1.000)\n  Text: 'bad'"
    );
}

#[test]
fn message_orchestrator_applies_the_threshold() {
    let config = config(Some(0.8_f64));
    let detections = vec![
        detection("pii", 0.5_f64, "email", None),
        detection("toxic", 0.9_f64, "bad", None),
    ];
    assert_eq!(
        orchestrator_message(&config, &detections),
        "IBM Guardrail Detector failed: 1 violation(s) detected\n\n- TOXIC (detector: jailbreak, score: 0.900)\n  Text: 'bad'"
    );
}

#[test]
fn message_orchestrator_without_violations_reports_zero() {
    let config = config(None);
    assert_eq!(
        orchestrator_message(&config, &[]),
        "IBM Guardrail Detector failed: 0 violation(s) detected"
    );
}
