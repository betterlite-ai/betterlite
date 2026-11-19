#![cfg(test)]

use std::collections::BTreeMap;

use crate::finops::vantage::support::{self, Recorder};

#[test]
fn the_frequency_defaults_to_hourly() {
    let logger = support::build(
        &BTreeMap::new(),
        support::params(),
        &Recorder::new(false),
        None,
    );
    assert_eq!(logger.settings.frequency, "hourly");
    assert_eq!(logger.focus.frequency, "hourly");
}

#[test]
fn the_environment_frequency_is_lowercased() {
    let environment = support::environment(&[("VANTAGE_EXPORT_FREQUENCY", "InTeRvAl")]);
    let logger = support::build(&environment, support::params(), &Recorder::new(false), None);
    assert_eq!(logger.settings.frequency, "interval");
    assert_eq!(logger.focus.frequency, "interval");
}

#[test]
fn a_parameter_overrides_the_environment_frequency() {
    let environment = support::environment(&[("VANTAGE_EXPORT_FREQUENCY", "daily")]);
    let mut params = support::params();
    params.frequency = Some("Interval".to_owned());
    let logger = support::build(&environment, params, &Recorder::new(false), None);
    assert_eq!(logger.settings.frequency, "interval");
    assert_eq!(logger.focus.frequency, "interval");
}

#[test]
fn an_empty_environment_frequency_takes_the_default() {
    let environment = support::environment(&[("VANTAGE_EXPORT_FREQUENCY", "")]);
    let logger = support::build(&environment, support::params(), &Recorder::new(false), None);
    assert_eq!(logger.settings.frequency, "hourly");
    assert_eq!(logger.focus.frequency, "hourly");
}
