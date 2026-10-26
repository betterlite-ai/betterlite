#![cfg(test)]

use super::{CloudZeroExportRequest, CloudZeroInitRequest};

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
