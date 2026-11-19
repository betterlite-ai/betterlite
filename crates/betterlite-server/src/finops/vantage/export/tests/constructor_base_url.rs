#![cfg(test)]

use std::collections::BTreeMap;

use crate::finops::vantage::support::{self, Recorder};

#[test]
fn the_base_url_defaults_to_the_vantage_api() {
    let logger = support::build(
        &BTreeMap::new(),
        support::params(),
        &Recorder::new(false),
        None,
    );
    assert_eq!(logger.settings.base_url, "https://api.vantage.sh");
    assert_eq!(
        logger.focus.destination_config.base_url,
        Some("https://api.vantage.sh".to_owned())
    );
}

#[test]
fn the_environment_sets_the_base_url() {
    let environment = support::environment(&[("VANTAGE_BASE_URL", "https://vantage.example")]);
    let logger = support::build(&environment, support::params(), &Recorder::new(false), None);
    assert_eq!(logger.settings.base_url, "https://vantage.example");
    assert_eq!(
        logger.focus.destination_config.base_url,
        Some("https://vantage.example".to_owned())
    );
}

#[test]
fn a_parameter_overrides_the_environment_base_url() {
    let environment = support::environment(&[("VANTAGE_BASE_URL", "https://vantage.example")]);
    let mut params = support::params();
    params.base_url = Some("https://parameter.example".to_owned());
    let logger = support::build(&environment, params, &Recorder::new(false), None);
    assert_eq!(logger.settings.base_url, "https://parameter.example");
    assert_eq!(
        logger.focus.destination_config.base_url,
        Some("https://parameter.example".to_owned())
    );
}

#[test]
fn an_explicit_empty_base_url_is_used_as_given() {
    let mut params = support::params();
    params.base_url = Some(String::new());
    let logger = support::build(&BTreeMap::new(), params, &Recorder::new(false), None);
    assert_eq!(logger.settings.base_url, "");
    assert_eq!(
        logger.focus.destination_config.base_url,
        Some(String::new())
    );
}

#[test]
fn an_empty_environment_base_url_takes_the_default() {
    let environment = support::environment(&[("VANTAGE_BASE_URL", "")]);
    let logger = support::build(&environment, support::params(), &Recorder::new(false), None);
    assert_eq!(logger.settings.base_url, "https://api.vantage.sh");
    assert_eq!(
        logger.focus.destination_config.base_url,
        Some("https://api.vantage.sh".to_owned())
    );
}
