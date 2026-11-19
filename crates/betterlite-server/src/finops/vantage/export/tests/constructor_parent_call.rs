#![cfg(test)]

use std::collections::BTreeMap;

use crate::finops::vantage::error::VantageExportError;
use crate::finops::vantage::export::VantageExportLogger;
use crate::finops::vantage::support::{self, ExportCall, FIXED_NOW, Recorder, at};

#[test]
fn the_parent_runs_as_vantage_in_csv_with_the_vantage_prefix() {
    let logger = support::build(
        &BTreeMap::new(),
        support::params(),
        &Recorder::new(false),
        None,
    );
    assert_eq!(logger.focus.provider, "vantage");
    assert_eq!(logger.focus.export_format, "csv");
    assert_eq!(logger.focus.prefix, "vantage_exports");
}

#[test]
fn focus_environment_values_do_not_override_the_explicit_parent_values() {
    let environment = support::environment(&[
        ("FOCUS_PROVIDER", "gcs"),
        ("FOCUS_FORMAT", "parquet"),
        ("FOCUS_FREQUENCY", "weekly"),
        ("FOCUS_PREFIX", "focus_exports"),
    ]);
    let logger = support::build(&environment, support::params(), &Recorder::new(false), None);
    assert_eq!(logger.focus.provider, "vantage");
    assert_eq!(logger.focus.export_format, "csv");
    assert_eq!(logger.focus.frequency, "hourly");
    assert_eq!(logger.focus.prefix, "vantage_exports");
}

#[test]
fn the_parent_still_reads_the_focus_cron_offset() {
    let environment = support::environment(&[("FOCUS_CRON_OFFSET", "7")]);
    let logger = support::build(&environment, support::params(), &Recorder::new(false), None);
    assert_eq!(logger.focus.cron_offset_minute, 7);
}

#[test]
fn the_cron_offset_parameter_overrides_the_focus_environment() {
    let environment = support::environment(&[("FOCUS_CRON_OFFSET", "7")]);
    let mut params = support::params();
    params.cron_offset_minute = Some(9);
    let logger = support::build(&environment, params, &Recorder::new(false), None);
    assert_eq!(logger.focus.cron_offset_minute, 9);
}

#[tokio::test]
async fn the_export_operation_is_the_parent_export_behind_the_dyn_trait() {
    let recorder = Recorder::new(false);
    let logger = support::build(&BTreeMap::new(), support::params(), &recorder, None);
    let export: &dyn VantageExportLogger = &logger;
    export
        .export_usage_data(None, None, None)
        .await
        .expect("the fake export succeeds");
    assert_eq!(recorder.calls(), vec![ExportCall::All { limit: None }]);
}

#[tokio::test]
async fn an_unpaired_window_is_refused_before_the_engine_runs() {
    let recorder = Recorder::new(false);
    let logger = support::build(&BTreeMap::new(), support::params(), &recorder, None);
    let export: &dyn VantageExportLogger = &logger;
    let error = export
        .export_usage_data(None, Some(at(FIXED_NOW)), None)
        .await
        .expect_err("one bound alone is refused");
    assert!(matches!(error, VantageExportError::UnpairedBounds));
    assert!(recorder.calls().is_empty());
}

#[test]
fn the_destination_config_carries_the_resolved_base_url() {
    let logger = support::build(
        &BTreeMap::new(),
        support::params(),
        &Recorder::new(false),
        None,
    );
    assert_eq!(
        logger.focus.destination_config.base_url,
        Some("https://api.vantage.sh".to_owned())
    );
    assert!(logger.focus.destination_config.bucket_name.is_none());
    assert!(
        logger
            .focus
            .destination_config
            .service_account_json
            .is_none()
    );
}
