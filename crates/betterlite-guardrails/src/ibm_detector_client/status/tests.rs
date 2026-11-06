#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_config::tls::Verification;
use reqwest::StatusCode;
use secrecy::SecretString;

use super::{detector_server_status, orchestrator_status};
use crate::content_filter::risk_scoring::GuardrailStatus;
use crate::ibm_detector_client::client::error::IbmClientError;
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
    }
}

fn detection(score: f64) -> Detection {
    Detection {
        start: None,
        end: None,
        text: None,
        detection: None,
        detection_type: Some("toxic".to_owned()),
        score: Some(score),
        evidences: None,
        metadata: None,
        detector_id: None,
    }
}

fn failed_call() -> IbmClientError {
    IbmClientError::Status {
        status: StatusCode::INTERNAL_SERVER_ERROR,
    }
}

fn malformed_response() -> IbmClientError {
    IbmClientError::ResponseJson(serde_json::from_str::<i32>("not json").expect_err("invalid json"))
}

#[test]
fn detector_server_status_without_detections_is_success() {
    let config = config(None);
    assert_eq!(
        detector_server_status(&config, &Ok(vec![Vec::new(), Vec::new()])),
        GuardrailStatus::Success
    );
    assert_eq!(
        detector_server_status(&config, &Ok(Vec::new())),
        GuardrailStatus::Success
    );
}

#[test]
fn detector_server_status_with_detections_is_intervened() {
    let config = config(None);
    let outcome = Ok(vec![Vec::new(), vec![detection(0.9_f64)]]);
    assert_eq!(
        detector_server_status(&config, &outcome),
        GuardrailStatus::GuardrailIntervened
    );
}

#[test]
fn detector_server_status_applies_the_threshold() {
    let config = config(Some(0.5_f64));
    let outcome = Ok(vec![vec![detection(0.4_f64)]]);
    assert_eq!(
        detector_server_status(&config, &outcome),
        GuardrailStatus::Success
    );
}

#[test]
fn detector_server_status_of_a_failed_call_is_failed_to_respond() {
    let config = config(None);
    assert_eq!(
        detector_server_status(&config, &Err(failed_call())),
        GuardrailStatus::GuardrailFailedToRespond
    );
}

#[test]
fn detector_server_status_of_a_malformed_response_is_failed_to_respond() {
    let config = config(None);
    assert_eq!(
        detector_server_status(&config, &Err(malformed_response())),
        GuardrailStatus::GuardrailFailedToRespond
    );
}

#[test]
fn orchestrator_status_without_detections_is_success() {
    let config = config(None);
    assert_eq!(
        orchestrator_status(&config, &Ok(Vec::new())),
        GuardrailStatus::Success
    );
}

#[test]
fn orchestrator_status_with_detections_is_intervened() {
    let config = config(None);
    assert_eq!(
        orchestrator_status(&config, &Ok(vec![detection(0.9_f64)])),
        GuardrailStatus::GuardrailIntervened
    );
}

#[test]
fn orchestrator_status_applies_the_threshold() {
    let config = config(Some(0.5_f64));
    assert_eq!(
        orchestrator_status(&config, &Ok(vec![detection(0.4_f64)])),
        GuardrailStatus::Success
    );
}

#[test]
fn orchestrator_status_of_a_failed_call_is_failed_to_respond() {
    let config = config(None);
    assert_eq!(
        orchestrator_status(&config, &Err(failed_call())),
        GuardrailStatus::GuardrailFailedToRespond
    );
}

#[test]
fn orchestrator_status_of_a_malformed_response_is_failed_to_respond() {
    let config = config(None);
    assert_eq!(
        orchestrator_status(&config, &Err(malformed_response())),
        GuardrailStatus::GuardrailFailedToRespond
    );
}
