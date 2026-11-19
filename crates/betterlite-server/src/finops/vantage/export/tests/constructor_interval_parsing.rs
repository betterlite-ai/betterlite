#![cfg(test)]

use std::collections::BTreeMap;

use tracing::Level;

use crate::finops::vantage::support::{self, Recorder};

#[test]
fn the_environment_interval_is_parsed() {
    let environment = support::environment(&[("VANTAGE_EXPORT_INTERVAL_SECONDS", "120")]);
    let logger = support::build(&environment, support::params(), &Recorder::new(false), None);
    assert_eq!(logger.settings.interval_seconds, Some(120));
    assert_eq!(logger.focus.interval_seconds, 120);
}

#[test]
fn an_invalid_environment_interval_warns_and_falls_back_to_the_parent_default() {
    let (logs, _guard) = support::capture_logs();
    let environment = support::environment(&[("VANTAGE_EXPORT_INTERVAL_SECONDS", "abc")]);
    let logger = support::build(&environment, support::params(), &Recorder::new(false), None);
    assert_eq!(logger.settings.interval_seconds, None);
    assert_eq!(logger.focus.interval_seconds, 60);
    assert!(logs.has_at(
        Level::WARN,
        "Invalid VANTAGE_EXPORT_INTERVAL_SECONDS value: abc, ignoring"
    ));
}

#[test]
fn the_focus_interval_applies_when_the_vantage_value_falls_back() {
    let environment = support::environment(&[
        ("VANTAGE_EXPORT_INTERVAL_SECONDS", "abc"),
        ("FOCUS_INTERVAL_SECONDS", "90"),
    ]);
    let logger = support::build(&environment, support::params(), &Recorder::new(false), None);
    assert_eq!(logger.settings.interval_seconds, None);
    assert_eq!(logger.focus.interval_seconds, 90);
}

#[test]
fn a_parameter_overrides_the_environment_interval() {
    let environment = support::environment(&[("VANTAGE_EXPORT_INTERVAL_SECONDS", "120")]);
    let mut params = support::params();
    params.interval_seconds = Some(30);
    let logger = support::build(&environment, params, &Recorder::new(false), None);
    assert_eq!(logger.settings.interval_seconds, Some(30));
    assert_eq!(logger.focus.interval_seconds, 30);
}

#[test]
fn an_explicit_zero_interval_is_used_as_given() {
    let mut params = support::params();
    params.interval_seconds = Some(0);
    let logger = support::build(&BTreeMap::new(), params, &Recorder::new(false), None);
    assert_eq!(logger.settings.interval_seconds, Some(0));
    assert_eq!(logger.focus.interval_seconds, 0);
}

#[test]
fn an_empty_environment_interval_is_unset_without_a_warning() {
    let (logs, _guard) = support::capture_logs();
    let environment = support::environment(&[("VANTAGE_EXPORT_INTERVAL_SECONDS", "")]);
    let logger = support::build(&environment, support::params(), &Recorder::new(false), None);
    assert_eq!(logger.settings.interval_seconds, None);
    assert_eq!(logger.focus.interval_seconds, 60);
    assert!(!logs.has_at(Level::WARN, "Invalid VANTAGE_EXPORT_INTERVAL_SECONDS"));
}
