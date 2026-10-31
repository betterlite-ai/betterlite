#![cfg(test)]

use super::FactoryError;
use crate::redis_connection_settings::error::RedisSettingsError;

#[test]
fn settings_errors_carry_their_message_through() {
    let error = FactoryError::from(RedisSettingsError::MissingTarget);
    assert_eq!(
        error.to_string(),
        "Either 'host' or 'url' must be specified for redis."
    );
}

#[test]
fn sentinel_requires_nodes_and_service_name_names_both_fields() {
    let error = FactoryError::SentinelTargetIncomplete;
    assert!(error.to_string().contains("sentinel_nodes"));
    assert!(error.to_string().contains("service_name"));
}

#[test]
fn invalid_reinitialize_steps_names_the_key_and_the_value() {
    let error = FactoryError::InvalidReinitializeSteps {
        value: "-1".to_owned(),
    };
    assert!(error.to_string().contains("reinitialize_steps"));
    assert!(error.to_string().contains("-1"));
}
