#![cfg(test)]

use betterlite_config::sections::capability_forecast as section;
use betterlite_router::capability_forecast::config::{Calibration, ResponseFormat};
use betterlite_router::capability_forecast::error::ForecastError;

use super::{deployment_configs, router_config};

fn configured() -> section::Config {
    section::Config {
        efficient_tier: " SIMPLE ".to_owned(),
        capable_tier: "REASONING".to_owned(),
        base_threshold: 0.6_f64,
        threshold_step: 0.05_f64,
        max_output_tokens: 512,
        calibration: Some(section::Calibration {
            version: "held-out-v1".to_owned(),
            slope: 1.5_f64,
            intercept: -0.25_f64,
        }),
        response_format: section::ResponseFormat::JsonObject,
    }
}

#[test]
fn every_section_field_reaches_the_router_config() {
    let converted = router_config(&configured()).unwrap();
    assert_eq!(converted.efficient_tier, "SIMPLE");
    assert_eq!(converted.capable_tier, "REASONING");
    assert_eq!(converted.base_threshold.to_bits(), 0.6_f64.to_bits());
    assert_eq!(converted.threshold_step.to_bits(), 0.05_f64.to_bits());
    assert_eq!(converted.max_output_tokens, 512);
    assert_eq!(converted.response_format, ResponseFormat::JsonObject);
    assert_eq!(
        converted.calibration,
        Some(Calibration {
            version: "held-out-v1".to_owned(),
            slope: 1.5_f64,
            intercept: -0.25_f64,
        })
    );
}

#[test]
fn an_absent_calibration_and_schema_mode_stay_absent() {
    let mut settings = configured();
    settings.calibration = None;
    settings.response_format = section::ResponseFormat::JsonSchema;
    let converted = router_config(&settings).unwrap();
    assert_eq!(converted.calibration, None);
    assert_eq!(converted.response_format, ResponseFormat::JsonSchema);
}

#[test]
fn an_invalid_section_value_is_refused_with_its_field() {
    let mut settings = configured();
    settings.base_threshold = 1.5_f64;
    let error = router_config(&settings).unwrap_err();
    assert!(matches!(error, ForecastError::BaseThreshold { .. }));
}

#[test]
fn deployment_indexes_keep_their_configuration() {
    let settings = section::Settings {
        deployments: vec![Some(configured()), None],
    };
    let converted = deployment_configs(&settings).unwrap();
    assert_eq!(converted.len(), 2);
    assert_eq!(
        converted[0].as_ref().map(|config| config.max_output_tokens),
        Some(512)
    );
    assert_eq!(converted[1], None);
}

#[test]
fn an_invalid_deployment_stops_the_whole_conversion() {
    let mut invalid = configured();
    invalid.capable_tier = "  ".to_owned();
    let settings = section::Settings {
        deployments: vec![Some(configured()), Some(invalid)],
    };
    assert!(matches!(
        deployment_configs(&settings).unwrap_err(),
        ForecastError::EmptyCapableTier
    ));
}
