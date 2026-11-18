#![cfg(test)]

use crate::finops::focus::support::{self, Recorder};

#[tokio::test]
async fn parameters_override_the_environment() {
    let recorder = Recorder::new(false);
    let mut params = support::params();
    params.provider = Some("GCS".to_owned());
    params.export_format = Some("CSV".to_owned());
    params.frequency = Some("Daily".to_owned());
    params.cron_offset_minute = Some(17);
    params.interval_seconds = Some(42);
    params.prefix = Some("Chosen".to_owned());
    params.destination_config = Some(support::overrides("bucket"));
    let logger = support::build(
        &support::environment(&[
            ("FOCUS_PROVIDER", "s3"),
            ("FOCUS_FORMAT", "parquet"),
            ("FOCUS_FREQUENCY", "hourly"),
            ("FOCUS_CRON_OFFSET", "5"),
            ("FOCUS_INTERVAL_SECONDS", "60"),
            ("FOCUS_PREFIX", "environment"),
        ]),
        params,
        recorder.factory(),
        None,
    );
    assert_eq!(logger.provider, "gcs");
    assert_eq!(logger.export_format, "csv");
    assert_eq!(logger.frequency, "daily");
    assert_eq!(logger.cron_offset_minute, 17);
    assert_eq!(logger.interval_seconds, 42);
    assert_eq!(logger.prefix, "Chosen");
    logger.export_usage_data(None, None, None).await.unwrap();
    let settings = recorder.settings();
    assert_eq!(settings.len(), 1);
    assert_eq!(settings[0].provider, "gcs");
    assert_eq!(settings[0].export_format, "csv");
    assert_eq!(settings[0].prefix, "Chosen");
    assert_eq!(settings[0].bucket_name.as_deref(), Some("bucket"));
}

#[test]
fn a_parameter_wins_over_an_unreadable_interval() {
    let recorder = Recorder::new(false);
    let mut params = support::params();
    params.interval_seconds = Some(30);
    let logger = support::build(
        &support::environment(&[("FOCUS_INTERVAL_SECONDS", "half")]),
        params,
        recorder.factory(),
        None,
    );
    assert_eq!(logger.interval_seconds, 30);
}
