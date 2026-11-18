#![cfg(test)]

use std::collections::BTreeMap;

use crate::finops::focus::error::FocusError;
use crate::finops::focus::support::{self, Recorder, at};

#[tokio::test]
async fn a_start_without_an_end_is_rejected_before_any_export() {
    let recorder = Recorder::new(false);
    let logger = support::build(
        &BTreeMap::new(),
        support::params(),
        recorder.factory(),
        None,
    );
    let result = logger
        .export_usage_data(None, Some(at("2024-03-09T00:00:00Z")), None)
        .await;
    let Err(error) = result else {
        panic!("a start without an end must fail");
    };
    assert!(matches!(&error, FocusError::UnpairedBounds));
    assert_eq!(
        error.to_string(),
        "start_time_utc and end_time_utc must be provided together"
    );
    assert!(recorder.settings().is_empty());
    assert!(recorder.calls().is_empty());
}

#[tokio::test]
async fn an_end_without_a_start_is_rejected_before_any_export() {
    let recorder = Recorder::new(false);
    let logger = support::build(
        &BTreeMap::new(),
        support::params(),
        recorder.factory(),
        None,
    );
    let result = logger
        .export_usage_data(Some(5), None, Some(at("2024-03-10T00:00:00Z")))
        .await;
    let Err(error) = result else {
        panic!("an end without a start must fail");
    };
    assert!(matches!(&error, FocusError::UnpairedBounds));
    assert!(recorder.settings().is_empty());
    assert!(recorder.calls().is_empty());
}
