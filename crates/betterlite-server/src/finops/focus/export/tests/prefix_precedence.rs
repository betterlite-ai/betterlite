#![cfg(test)]

use std::collections::BTreeMap;

use crate::finops::focus::support::{self, Recorder};

#[test]
fn a_parameter_wins_over_the_environment_and_the_default() {
    let recorder = Recorder::new(false);
    let mut params = support::params();
    params.prefix = Some("parameter".to_owned());
    let logger = support::build(
        &support::environment(&[("FOCUS_PREFIX", "environment")]),
        params,
        recorder.factory(),
        None,
    );
    assert_eq!(logger.prefix, "parameter");
}

#[test]
fn the_environment_wins_over_the_default() {
    let recorder = Recorder::new(false);
    let logger = support::build(
        &support::environment(&[("FOCUS_PREFIX", "environment")]),
        support::params(),
        recorder.factory(),
        None,
    );
    assert_eq!(logger.prefix, "environment");
}

#[test]
fn an_unset_or_empty_prefix_reads_as_focus_exports() {
    let recorder = Recorder::new(false);
    let unset = support::build(
        &BTreeMap::new(),
        support::params(),
        recorder.factory(),
        None,
    );
    assert_eq!(unset.prefix, "focus_exports");
    let empty = support::build(
        &support::environment(&[("FOCUS_PREFIX", "")]),
        support::params(),
        recorder.factory(),
        None,
    );
    assert_eq!(empty.prefix, "focus_exports");
}
