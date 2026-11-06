#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_config::tls::Verification;
use secrecy::SecretString;

use super::filter_detections;
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

fn scored(score: Option<f64>) -> Detection {
    Detection {
        start: None,
        end: None,
        text: None,
        detection: None,
        detection_type: None,
        score,
        evidences: None,
        metadata: None,
        detector_id: None,
    }
}

fn kept_scores(config: &IbmDetectorConfig, scores: &[Option<f64>]) -> Vec<Option<f64>> {
    let detections: Vec<Detection> = scores.iter().copied().map(scored).collect();
    filter_detections(config, &detections)
        .into_iter()
        .map(|detection| detection.score)
        .collect()
}

#[test]
fn threshold_absent_accepts_every_detection() {
    let config = config(None);
    assert_eq!(
        kept_scores(&config, &[Some(0.0_f64), Some(0.5_f64), None]),
        [Some(0.0_f64), Some(0.5_f64), None]
    );
}

#[test]
fn threshold_zero_keeps_every_non_negative_detection() {
    let config = config(Some(0.0_f64));
    assert_eq!(
        kept_scores(
            &config,
            &[Some(-0.1_f64), Some(0.0_f64), None, Some(0.7_f64)]
        ),
        [Some(0.0_f64), None, Some(0.7_f64)]
    );
}

#[test]
fn threshold_filters_detections_below_the_score() {
    let config = config(Some(0.8_f64));
    assert_eq!(
        kept_scores(&config, &[Some(0.79_f64), Some(0.8_f64), Some(1.0_f64)]),
        [Some(0.8_f64), Some(1.0_f64)]
    );
}
