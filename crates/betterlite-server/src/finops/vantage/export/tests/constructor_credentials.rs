#![cfg(test)]

use std::collections::BTreeMap;

use secrecy::{ExposeSecret as _, SecretString};

use crate::finops::vantage::support::{self, Recorder};

#[test]
fn absent_credentials_stay_out_of_the_destination_config() {
    let logger = support::build(
        &BTreeMap::new(),
        support::params(),
        &Recorder::new(false),
        None,
    );
    assert!(logger.settings.api_key.is_none());
    assert!(logger.settings.integration_token.is_none());
    assert!(logger.focus.destination_config.api_key.is_none());
    assert!(logger.focus.destination_config.integration_token.is_none());
}

#[test]
fn environment_credentials_reach_the_destination_config() {
    let environment = support::environment(&[
        ("VANTAGE_API_KEY", "environment-key"),
        ("VANTAGE_INTEGRATION_TOKEN", "environment-token"),
    ]);
    let logger = support::build(&environment, support::params(), &Recorder::new(false), None);
    assert_eq!(
        logger
            .focus
            .destination_config
            .api_key
            .as_ref()
            .map(|key| key.expose_secret().as_str()),
        Some("environment-key")
    );
    assert_eq!(
        logger
            .focus
            .destination_config
            .integration_token
            .as_ref()
            .map(|token| token.expose_secret().as_str()),
        Some("environment-token")
    );
}

#[test]
fn parameters_override_the_environment_credentials() {
    let environment = support::environment(&[
        ("VANTAGE_API_KEY", "environment-key"),
        ("VANTAGE_INTEGRATION_TOKEN", "environment-token"),
    ]);
    let mut params = support::params();
    params.api_key = Some(SecretString::from("parameter-key"));
    params.integration_token = Some(SecretString::from("parameter-token"));
    let logger = support::build(&environment, params, &Recorder::new(false), None);
    assert_eq!(
        logger
            .settings
            .integration_token
            .as_ref()
            .map(|token| token.expose_secret().as_str()),
        Some("parameter-token")
    );
    assert_eq!(
        logger
            .focus
            .destination_config
            .api_key
            .as_ref()
            .map(|key| key.expose_secret().as_str()),
        Some("parameter-key")
    );
    assert_eq!(
        logger
            .focus
            .destination_config
            .integration_token
            .as_ref()
            .map(|token| token.expose_secret().as_str()),
        Some("parameter-token")
    );
}

#[test]
fn an_explicit_empty_parameter_is_used_as_given() {
    let environment = support::environment(&[
        ("VANTAGE_API_KEY", "environment-key"),
        ("VANTAGE_INTEGRATION_TOKEN", "environment-token"),
    ]);
    let mut params = support::params();
    params.api_key = Some(SecretString::from(""));
    let logger = support::build(&environment, params, &Recorder::new(false), None);
    assert_eq!(
        logger
            .settings
            .api_key
            .as_ref()
            .map(|key| key.expose_secret().as_str()),
        Some("")
    );
    assert_eq!(
        logger
            .focus
            .destination_config
            .api_key
            .as_ref()
            .map(|key| key.expose_secret().as_str()),
        Some("")
    );
}

#[test]
fn an_empty_environment_credential_counts_as_unset() {
    let environment =
        support::environment(&[("VANTAGE_API_KEY", ""), ("VANTAGE_INTEGRATION_TOKEN", "")]);
    let logger = support::build(&environment, support::params(), &Recorder::new(false), None);
    assert!(logger.settings.api_key.is_none());
    assert!(logger.settings.integration_token.is_none());
    assert!(logger.focus.destination_config.api_key.is_none());
    assert!(logger.focus.destination_config.integration_token.is_none());
}
