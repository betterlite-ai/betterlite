#![cfg(test)]

use betterlite_db::budget_permission_rows::JsonData;
use rstest::rstest;
use rust_decimal::Decimal;

use crate::endpoints::passthrough_config_reads::caller::CallerAccess;
use crate::endpoints::passthrough_config_reads::db_read::PASS_THROUGH_FIELD;
use crate::endpoints::passthrough_config_reads::db_read::database_endpoints;
use crate::endpoints::passthrough_config_reads::record::EndpointRecordError;
use crate::endpoints::passthrough_config_reads::support::FakeSettings;
use crate::endpoints::passthrough_config_reads::support::SettingsReply;
use crate::endpoints::passthrough_config_reads::support::json;
use crate::endpoints::passthrough_config_reads::support::json_list;

const ENDPOINT_A: &str = r#"{"id": "a", "path": "/a", "target": "https://a"}"#;
const ENDPOINT_B: &str = r#"{"id": "b", "path": "/b", "target": "https://b"}"#;

#[rstest]
#[case::not_allowed(SettingsReply::NotAllowed)]
#[case::unavailable(SettingsReply::Unavailable)]
#[tokio::test]
async fn a_settings_read_failure_gives_no_endpoints(#[case] reply: SettingsReply) {
    let settings = FakeSettings::refusing(reply);
    let read = database_endpoints(&settings, None, None)
        .await
        .expect("no error");
    assert!(read.is_empty());
}

#[tokio::test]
async fn an_absent_stored_field_gives_no_endpoints() {
    let settings = FakeSettings::refusing(SettingsReply::Missing);
    let read = database_endpoints(&settings, None, None)
        .await
        .expect("no error");
    assert!(read.is_empty());
}

#[tokio::test]
async fn a_null_stored_field_gives_no_endpoints() {
    let settings = FakeSettings::returning(JsonData::Null);
    let read = database_endpoints(&settings, None, None)
        .await
        .expect("no error");
    assert!(read.is_empty());
}

#[tokio::test]
async fn every_stored_endpoint_is_returned() {
    let settings = FakeSettings::returning(json_list(&[ENDPOINT_A, ENDPOINT_B]));
    let read = database_endpoints(&settings, None, None)
        .await
        .expect("db endpoints");
    assert_eq!(read.len(), 2);
    assert_eq!(read[0].id.as_deref(), Some("a"));
    assert_eq!(read[0].path, "/a");
    assert_eq!(read[1].path, "/b");
    assert!(!read[0].is_from_config.0);
    assert!(!read[1].is_from_config.0);
}

#[tokio::test]
async fn a_stored_endpoint_takes_every_default() {
    let settings =
        FakeSettings::returning(json_list(&[r#"{"path": "/a", "target": "https://a"}"#]));
    let read = database_endpoints(&settings, None, None)
        .await
        .expect("db endpoints");
    let endpoint = &read[0];
    assert_eq!(endpoint.id, None);
    assert!(endpoint.headers.is_empty());
    assert!(endpoint.default_query_params.is_empty());
    assert!(!endpoint.include_subpath.0);
    assert_eq!(endpoint.cost_per_request, Decimal::ZERO);
    assert_eq!(endpoint.timeout, None);
    assert!(endpoint.auth.0);
    assert_eq!(endpoint.guardrails, None);
    assert_eq!(endpoint.methods, None);
}

#[tokio::test]
async fn stored_guardrail_settings_keep_their_null_defaults() {
    let settings = FakeSettings::returning(json_list(&[
        r#"{"path": "/a", "target": "https://a", "guardrails": {"pii": null}}"#,
        r#"{"path": "/b", "target": "https://b", "guardrails": {"pii": {}}}"#,
    ]));
    let read = database_endpoints(&settings, None, None)
        .await
        .expect("db endpoints");
    assert_eq!(read.len(), 2);
    let null_settings = read[0].guardrails.as_ref().expect("guardrails");
    assert!(null_settings.get("pii").expect("the pii entry").is_none());
    let empty_settings = read[1]
        .guardrails
        .as_ref()
        .expect("guardrails")
        .get("pii")
        .expect("the pii entry")
        .as_ref()
        .expect("the pii settings");
    assert_eq!(empty_settings.request_fields, None);
    assert_eq!(empty_settings.response_fields, None);
}

#[tokio::test]
async fn a_stored_from_config_flag_is_ignored() {
    let settings = FakeSettings::returning(json_list(&[
        r#"{"id": "a", "path": "/a", "target": "https://a", "is_from_config": true}"#,
    ]));
    let read = database_endpoints(&settings, None, None)
        .await
        .expect("db endpoints");
    assert!(!read[0].is_from_config.0);
}

#[rstest]
#[case::object(json(r#"{"path": "/a"}"#))]
#[case::text(json(r#""text""#))]
#[case::number(json("7"))]
#[case::flag(json("true"))]
#[tokio::test]
async fn a_stored_field_that_is_not_a_list_is_an_error(#[case] value: JsonData) {
    let settings = FakeSettings::returning(value);
    let error = database_endpoints(&settings, None, None)
        .await
        .expect_err("not a list");
    assert!(matches!(error, EndpointRecordError::NotAList));
}

#[tokio::test]
async fn a_malformed_stored_endpoint_is_an_error() {
    let settings = FakeSettings::returning(json_list(&[r#"{"path": "/broken"}"#]));
    let error = database_endpoints(&settings, None, None)
        .await
        .expect_err("a malformed record");
    assert!(matches!(error, EndpointRecordError::Decode(_)));
}

#[tokio::test]
async fn a_non_object_stored_entry_is_skipped() {
    let settings = FakeSettings::returning(json_list(&[r#""text""#, ENDPOINT_A]));
    let read = database_endpoints(&settings, None, None)
        .await
        .expect("db endpoints");
    assert_eq!(read.len(), 1);
    assert_eq!(read[0].path, "/a");
}

#[tokio::test]
async fn an_endpoint_id_selects_the_matching_endpoint() {
    let settings = FakeSettings::returning(json_list(&[ENDPOINT_A, ENDPOINT_B]));
    let read = database_endpoints(&settings, Some("b"), None)
        .await
        .expect("db endpoints");
    assert_eq!(read.len(), 1);
    assert_eq!(read[0].id.as_deref(), Some("b"));
    assert_eq!(read[0].path, "/b");
}

#[tokio::test]
async fn an_unknown_endpoint_id_gives_no_endpoints() {
    let settings = FakeSettings::returning(json_list(&[ENDPOINT_A, ENDPOINT_B]));
    let read = database_endpoints(&settings, Some("missing"), None)
        .await
        .expect("db endpoints");
    assert!(read.is_empty());
}

#[tokio::test]
async fn an_endpoint_without_an_id_never_matches() {
    let settings =
        FakeSettings::returning(json_list(&[r#"{"path": "/a", "target": "https://a"}"#]));
    let read = database_endpoints(&settings, Some("a"), None)
        .await
        .expect("db endpoints");
    assert!(read.is_empty());
}

#[tokio::test]
async fn a_malformed_entry_before_the_match_is_an_error() {
    let settings = FakeSettings::returning(json_list(&[r#"{"path": "/broken"}"#, ENDPOINT_B]));
    let error = database_endpoints(&settings, Some("b"), None)
        .await
        .expect_err("a malformed record");
    assert!(matches!(error, EndpointRecordError::Decode(_)));
}

#[tokio::test]
async fn a_malformed_entry_after_the_match_is_not_read() {
    let settings = FakeSettings::returning(json_list(&[ENDPOINT_B, r#"{"path": "/broken"}"#]));
    let read = database_endpoints(&settings, Some("b"), None)
        .await
        .expect("db endpoints");
    assert_eq!(read.len(), 1);
    assert_eq!(read[0].path, "/b");
}

#[tokio::test]
async fn the_stored_field_is_read_by_its_litellm_name_as_a_full_admin() {
    let settings = FakeSettings::returning(json_list(&[ENDPOINT_A]));
    database_endpoints(&settings, None, None)
        .await
        .expect("db endpoints");
    assert_eq!(
        settings.called(),
        vec![(PASS_THROUGH_FIELD.to_owned(), CallerAccess::FullAdmin)]
    );
}

#[tokio::test]
async fn the_callers_access_reaches_the_reader() {
    let settings = FakeSettings::returning(json_list(&[ENDPOINT_A]));
    database_endpoints(&settings, None, Some(CallerAccess::AdminView))
        .await
        .expect("db endpoints");
    assert_eq!(
        settings.called(),
        vec![(PASS_THROUGH_FIELD.to_owned(), CallerAccess::AdminView)]
    );
}
