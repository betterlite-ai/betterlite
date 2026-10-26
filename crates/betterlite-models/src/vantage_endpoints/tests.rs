#![cfg(test)]

use super::{
    VantageDryRunRequest, VantageExportRequest, VantageInitRequest, VantageSettingsUpdate,
};

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
    assert_eq!(update.api_key, None);
}

#[test]
fn finops_management_vantage_accepts_non_empty_update_credentials() {
    let update: VantageSettingsUpdate =
        serde_json::from_str(r#"{"api_key":"new-key","integration_token":"new-token"}"#)
            .expect("non-empty credentials are valid");
    assert_eq!(update.api_key.as_deref(), Some("new-key"));
    assert_eq!(update.integration_token.as_deref(), Some("new-token"));
}
