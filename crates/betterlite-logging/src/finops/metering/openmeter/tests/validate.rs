#![cfg(test)]

use super::support::API_KEY;
use crate::finops::metering::openmeter::settings::{
    DEFAULT_ENDPOINT, DEFAULT_EVENT_TYPE, OpenMeterSettings, OpenMeterSettingsError,
};
use crate::finops::metering::support::variables;

#[test]
fn validate_reports_the_missing_api_key() {
    let error = OpenMeterSettings::from_environment(&variables(&[])).unwrap_err();
    let OpenMeterSettingsError::MissingKeys { missing_keys } = error else {
        panic!("the settings error is not MissingKeys");
    };
    assert_eq!(missing_keys, vec!["OPENMETER_API_KEY"]);
}

#[test]
fn validate_counts_an_empty_api_key_as_unset() {
    let error =
        OpenMeterSettings::from_environment(&variables(&[("OPENMETER_API_KEY", "")])).unwrap_err();
    let OpenMeterSettingsError::MissingKeys { missing_keys } = error else {
        panic!("the settings error is not MissingKeys");
    };
    assert_eq!(missing_keys, vec!["OPENMETER_API_KEY"]);
}

#[test]
fn validate_accepts_a_key_and_fills_the_optional_defaults() {
    let settings =
        OpenMeterSettings::from_environment(&variables(&[("OPENMETER_API_KEY", API_KEY)])).unwrap();
    assert_eq!(settings.endpoint, DEFAULT_ENDPOINT);
    assert_eq!(settings.event_type, DEFAULT_EVENT_TYPE);
    assert!(settings.trust_request_user);
}

#[test]
fn validate_reports_an_unreadable_trust_value() {
    let environment = variables(&[
        ("OPENMETER_API_KEY", API_KEY),
        ("OPENMETER_TRUST_REQUEST_USER", "maybe"),
    ]);
    let error = OpenMeterSettings::from_environment(&environment).unwrap_err();
    let OpenMeterSettingsError::Environment(source) = error else {
        panic!("the settings error is not Environment");
    };
    assert_eq!(
        source.to_string(),
        "environment variable OPENMETER_TRUST_REQUEST_USER must be a bool"
    );
}
