use super::*;

#[tokio::test]
async fn cloudzero_schedule() {
    let doubled = settings_from(&[("CLOUDZERO_EXPORT_INTERVAL_MINUTES", "9223372036854775807")]);
    let doubled_logger = logger(doubled, datastore(), FakeTransport::default(), None);
    let error = hourly_usage_data_export(&doubled_logger)
        .await
        .expect_err("a doubled interval overflows");
    assert!(matches!(error, CloudZeroExportError::WindowOutOfRange));
    assert_eq!(
        error.to_string(),
        "the export window is outside the supported date range"
    );
    let too_long = settings_from(&[("CLOUDZERO_EXPORT_INTERVAL_MINUTES", "4611686018427387903")]);
    let too_long_logger = logger(too_long, datastore(), FakeTransport::default(), None);
    let too_long_error = hourly_usage_data_export(&too_long_logger)
        .await
        .expect_err("the lookback exceeds the duration range");
    assert!(matches!(
        too_long_error,
        CloudZeroExportError::WindowOutOfRange
    ));
}
