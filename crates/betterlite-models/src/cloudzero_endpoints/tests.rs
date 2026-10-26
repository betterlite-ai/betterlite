#![cfg(test)]

use super::{CloudZeroExportRequest, CloudZeroInitRequest};
use rstest::rstest;

#[test]
fn finops_management_cloudzero_defaults() {
    let init: CloudZeroInitRequest =
        serde_json::from_str(r#"{"api_key":"key","connection_id":"connection"}"#)
            .expect("valid init request");
    assert_eq!(init.timezone, "UTC");

    let export: CloudZeroExportRequest = serde_json::from_str("{}").expect("valid export request");
    assert_eq!(export.limit, None);
    assert_eq!(export.operation, "replace_hourly");
}

#[test]
fn finops_management_cloudzero_accepts_arbitrary_operation() {
    let export: CloudZeroExportRequest = serde_json::from_str(r#"{"operation":"future_mode"}"#)
        .expect("operation is intentionally open-ended");
    assert_eq!(export.operation, "future_mode");
}

#[rstest]
#[case::rfc3339(r#""2024-01-15T00:00:00Z""#, 1_705_276_800)]
#[case::naive(r#""2024-01-15T00:00:00""#, 1_705_276_800)]
#[case::without_seconds(r#""2024-01-15T00:00""#, 1_705_276_800)]
#[case::unix_timestamp("1705276800", 1_705_276_800)]
#[case::millisecond_timestamp("1705276800000", 1_705_276_800)]
#[case::float_timestamp("1705276800.5", 1_705_276_800)]
#[case::date_only(r#""2024-01-15""#, 1_705_276_800)]
fn finops_management_cloudzero_accepts_supported_datetime_forms(
    #[case] value: &str,
    #[case] expected_timestamp: i64,
) {
    let request: CloudZeroExportRequest =
        serde_json::from_str(&format!(r#"{{"start_time_utc":{value}}}"#))
            .expect("supported datetime form");
    assert_eq!(
        request.start_time_utc.expect("start time").timestamp(),
        expected_timestamp
    );
}
