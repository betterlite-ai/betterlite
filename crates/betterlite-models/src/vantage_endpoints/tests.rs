#![cfg(test)]

use super::{
    VantageDryRunRequest, VantageExportRequest, VantageInitRequest, VantageSettingsUpdate,
};
use rstest::rstest;

#[test]
fn finops_management_vantage_defaults() {
    let init: VantageInitRequest =
        serde_json::from_str(r#"{"api_key":"key","integration_token":"token"}"#)
            .expect("valid init request");
    assert_eq!(init.base_url, "https://api.vantage.sh");

    let dry_run: VantageDryRunRequest = serde_json::from_str("{}").expect("valid dry run request");
    assert_eq!(dry_run.limit, Some(500));

    let export: VantageExportRequest = serde_json::from_str("{}").expect("valid export request");
    assert_eq!(export.limit, None);
}

#[test]
fn finops_management_vantage_rejects_blank_credentials() {
    let init = serde_json::from_str::<VantageInitRequest>(
        r#"{"api_key":"  ","integration_token":"token"}"#,
    );
    init.unwrap_err();

    let update = serde_json::from_str::<VantageSettingsUpdate>(r#"{"integration_token":"\t"}"#);
    update.unwrap_err();
}

#[test]
fn finops_management_vantage_allows_null_update_credentials() {
    let update: VantageSettingsUpdate = serde_json::from_str(r#"{"api_key":null}"#)
        .expect("null means leave the stored credential unchanged");
    assert!(update.api_key.is_none());
}

#[test]
fn finops_management_vantage_accepts_non_empty_update_credentials() {
    let update: VantageSettingsUpdate =
        serde_json::from_str(r#"{"api_key":"new-key","integration_token":"new-token"}"#)
            .expect("non-empty credentials are valid");
    assert_eq!(
        update
            .api_key
            .as_ref()
            .map(secrecy::ExposeSecret::expose_secret),
        Some("new-key")
    );
    assert_eq!(
        update
            .integration_token
            .as_ref()
            .map(secrecy::ExposeSecret::expose_secret),
        Some("new-token")
    );
}

#[rstest]
#[case::rfc3339(r#""2024-01-15T00:00:00Z""#, 1_705_276_800)]
#[case::naive(r#""2024-01-15T00:00:00""#, 1_705_276_800)]
#[case::unix_timestamp("1705276800", 1_705_276_800)]
fn finops_management_vantage_accepts_supported_datetime_forms(
    #[case] value: &str,
    #[case] expected_timestamp: i64,
) {
    let request: VantageExportRequest =
        serde_json::from_str(&format!(r#"{{"start_time_utc":{value}}}"#))
            .expect("supported datetime form");
    assert_eq!(
        request.start_time_utc.expect("start time").timestamp(),
        expected_timestamp
    );
}
