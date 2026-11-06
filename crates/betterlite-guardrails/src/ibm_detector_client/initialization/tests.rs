#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_config::sections::ibm_guardrails::Configured;
use betterlite_config::tls::Verification;
use rstest::rstest;
use secrecy::SecretString;

use super::{IbmDetectorConfig, InitializationError};

fn configured() -> Configured {
    Configured {
        guardrail_name: "ibm".to_owned(),
        auth_token: Some(SecretString::from("token")),
        base_url: Some("https://detector.test".to_owned()),
        detector_id: Some("jailbreak".to_owned()),
        is_detector_server: true,
        detector_params: BTreeMap::new(),
        extra_headers: BTreeMap::new(),
        score_threshold: None,
        block_on_detection: true,
        ssl_verification: Verification::Enabled,
    }
}

#[rstest]
#[case::absent(None)]
#[case::empty(Some(""))]
fn missing_required_field_auth_token(#[case] token: Option<&str>) {
    let mut settings = configured();
    settings.auth_token = token.map(SecretString::from);
    let error = IbmDetectorConfig::try_from(settings).expect_err("missing token");
    assert!(matches!(error, InitializationError::MissingAuthToken));
}

#[rstest]
#[case::absent(None)]
#[case::empty(Some(""))]
fn missing_required_field_base_url(#[case] url: Option<&str>) {
    let mut settings = configured();
    settings.base_url = url.map(ToOwned::to_owned);
    let error = IbmDetectorConfig::try_from(settings).expect_err("missing base url");
    assert!(matches!(error, InitializationError::MissingBaseUrl));
}

#[rstest]
#[case::absent(None)]
#[case::empty(Some(""))]
fn missing_required_field_detector_id(#[case] detector_id: Option<&str>) {
    let mut settings = configured();
    settings.detector_id = detector_id.map(ToOwned::to_owned);
    let error = IbmDetectorConfig::try_from(settings).expect_err("missing detector id");
    assert!(matches!(error, InitializationError::MissingDetectorId));
}

#[test]
fn initialization_keeps_every_setting() {
    let mut settings = configured();
    settings.guardrail_name = "named".to_owned();
    settings.is_detector_server = false;
    settings.score_threshold = Some(0.4_f64);
    settings.block_on_detection = false;
    settings.ssl_verification = Verification::Disabled;
    settings
        .extra_headers
        .insert("X-Tenant".to_owned(), "acme".to_owned());
    let config = IbmDetectorConfig::try_from(settings).expect("config");
    assert_eq!(config.guardrail_name, "named");
    assert_eq!(config.detector_id, "jailbreak");
    assert!(!config.is_detector_server);
    assert_eq!(config.score_threshold, Some(0.4_f64));
    assert!(!config.block_on_detection);
    assert_eq!(config.ssl_verification, Verification::Disabled);
    assert_eq!(config.extra_headers["X-Tenant"], "acme");
}
