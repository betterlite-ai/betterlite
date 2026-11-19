#![cfg(test)]

use std::collections::BTreeMap;

use secrecy::ExposeSecret;

use crate::finops::vantage::support::{self, Recorder};

#[test]
fn an_empty_environment_takes_the_vantage_defaults() {
    let logger = support::build(
        &BTreeMap::new(),
        support::params(),
        &Recorder::new(false),
        None,
    );
    assert!(logger.settings.api_key.is_none());
    assert!(logger.settings.integration_token.is_none());
    assert_eq!(logger.settings.base_url, "https://api.vantage.sh");
    assert_eq!(logger.settings.frequency, "hourly");
    assert_eq!(logger.settings.interval_seconds, None);
}

#[test]
fn an_empty_environment_value_counts_as_unset_for_every_setting() {
    let environment = support::environment(&[
        ("VANTAGE_API_KEY", ""),
        ("VANTAGE_INTEGRATION_TOKEN", ""),
        ("VANTAGE_BASE_URL", ""),
        ("VANTAGE_EXPORT_FREQUENCY", ""),
        ("VANTAGE_EXPORT_INTERVAL_SECONDS", ""),
    ]);
    let logger = support::build(&environment, support::params(), &Recorder::new(false), None);
    assert!(logger.settings.api_key.is_none());
    assert!(logger.settings.integration_token.is_none());
    assert_eq!(logger.settings.base_url, "https://api.vantage.sh");
    assert_eq!(logger.settings.frequency, "hourly");
    assert_eq!(logger.settings.interval_seconds, None);
}

#[test]
fn the_environment_supplies_every_vantage_setting() {
    let environment = support::environment(&[
        ("VANTAGE_API_KEY", "environment-key"),
        ("VANTAGE_INTEGRATION_TOKEN", "environment-token"),
        ("VANTAGE_BASE_URL", "https://vantage.example"),
        ("VANTAGE_EXPORT_FREQUENCY", "DAILY"),
        ("VANTAGE_EXPORT_INTERVAL_SECONDS", "120"),
    ]);
    let logger = support::build(&environment, support::params(), &Recorder::new(false), None);
    assert_eq!(
        logger
            .settings
            .api_key
            .as_ref()
            .map(ExposeSecret::expose_secret),
        Some("environment-key")
    );
    assert_eq!(
        logger
            .settings
            .integration_token
            .as_ref()
            .map(ExposeSecret::expose_secret),
        Some("environment-token")
    );
    assert_eq!(logger.settings.base_url, "https://vantage.example");
    assert_eq!(logger.settings.frequency, "daily");
    assert_eq!(logger.settings.interval_seconds, Some(120));
    assert_eq!(logger.focus.frequency, "daily");
    assert_eq!(logger.focus.interval_seconds, 120);
}
