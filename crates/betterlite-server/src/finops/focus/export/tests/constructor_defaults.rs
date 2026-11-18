#![cfg(test)]

use std::collections::BTreeMap;

use crate::finops::focus::support::{self, Recorder};

#[test]
fn an_empty_environment_gives_the_documented_defaults_without_building_the_engine() {
    let recorder = Recorder::new(false);
    let logger = support::build(
        &BTreeMap::new(),
        support::params(),
        recorder.factory(),
        None,
    );
    assert_eq!(logger.provider, "s3");
    assert_eq!(logger.export_format, "parquet");
    assert_eq!(logger.frequency, "hourly");
    assert_eq!(logger.cron_offset_minute, 5);
    assert_eq!(logger.interval_seconds, None);
    assert_eq!(logger.prefix, "focus_exports");
    assert!(recorder.settings().is_empty());
}

#[test]
fn the_environment_supplies_the_three_names_in_lowercase() {
    let recorder = Recorder::new(false);
    let logger = support::build(
        &support::environment(&[
            ("FOCUS_PROVIDER", "VANTAGE"),
            ("FOCUS_FORMAT", "CSV"),
            ("FOCUS_FREQUENCY", "Daily"),
        ]),
        support::params(),
        recorder.factory(),
        None,
    );
    assert_eq!(logger.provider, "vantage");
    assert_eq!(logger.export_format, "csv");
    assert_eq!(logger.frequency, "daily");
}
