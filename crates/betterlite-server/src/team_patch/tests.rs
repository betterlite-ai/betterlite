#![cfg(test)]

mod fixtures;

use std::collections::BTreeMap;

use betterlite_config::sections::budget_reset::BudgetResetSettings;
use betterlite_db::budget_permission_rows::JsonData;
use chrono::{DateTime, NaiveTime, Utc};
use jiff::tz::TimeZoneDatabase;
use rstest::rstest;

use self::fixtures::TestDatabase;
use super::budget::BudgetResetError;
use super::budget::set_budget_reset_at;
use super::permission::update_object_permission;
use crate::key_policy_fields::KeyClock;
use crate::object_permissions::error::ObjectPermissionError;
use crate::team_request_contracts::shared::RequestProjection as _;
use crate::team_request_contracts::update::UpdateTeamRequest;

struct Clocks {
    settings: BudgetResetSettings,
    database: TimeZoneDatabase,
    now: DateTime<Utc>,
}

impl Clocks {
    fn at(now: &str) -> Self {
        Self {
            settings: BudgetResetSettings::default(),
            database: TimeZoneDatabase::from_env(),
            now: DateTime::parse_from_rfc3339(now)
                .unwrap()
                .with_timezone(&Utc),
        }
    }

    fn key(&self) -> KeyClock<'_> {
        KeyClock {
            now: self.now,
            reset: &self.settings,
            database: &self.database,
        }
    }
}

fn json(text: &str) -> JsonData {
    serde_json::from_str(text).unwrap()
}

fn update_request(document: &str) -> UpdateTeamRequest {
    UpdateTeamRequest::from_value(&json(document)).unwrap()
}

// the caller's fields as B806 hands them over: the exclude-unset dump LiteLLM calls updated_kv
fn updated(data: &UpdateTeamRequest) -> BTreeMap<String, JsonData> {
    let JsonData::Object(fields) = data.supplied().unwrap() else {
        panic!("a team update projects to an object");
    };
    fields
}

fn payload(document: &str) -> BTreeMap<String, JsonData> {
    let JsonData::Object(fields) = json(document) else {
        panic!("a team update is a JSON object");
    };
    fields
}

#[test]
fn budget_duration_absent_preserves_reset() {
    let request = update_request(r#"{"team_id":"t","team_alias":"kept","max_budget":10.0}"#);
    let mut updated_kv = updated(&request);
    let before = updated_kv.clone();
    set_budget_reset_at(
        &request.fields,
        &mut updated_kv,
        Clocks::at("2026-05-01T10:00:00Z").key(),
    )
    .unwrap();
    assert_eq!(updated_kv, before);
}

#[test]
fn budget_duration_null_clears_reset() {
    let request = update_request(r#"{"team_id":"t","budget_duration":null}"#);
    let mut updated_kv = updated(&request);
    assert_eq!(updated_kv.get("budget_duration"), Some(&JsonData::Null));
    set_budget_reset_at(
        &request.fields,
        &mut updated_kv,
        Clocks::at("2026-05-01T10:00:00Z").key(),
    )
    .unwrap();
    assert_eq!(updated_kv.get("budget_reset_at"), Some(&JsonData::Null));
    assert_eq!(updated_kv.get("budget_duration"), Some(&JsonData::Null));
}

#[test]
fn duration_uses_reset_clock() {
    let mut clocks = Clocks::at("2026-05-01T10:00:00Z");
    let request = update_request(r#"{"team_id":"t","budget_duration":"1d"}"#);
    let mut updated_kv = updated(&request);
    set_budget_reset_at(&request.fields, &mut updated_kv, clocks.key()).unwrap();
    assert_eq!(
        updated_kv.get("budget_reset_at"),
        Some(&JsonData::String("2026-05-02T00:00:00Z".to_owned()))
    );

    // the same request under a 06:00 reset in Asia/Kolkata lands half an hour into the next UTC day
    clocks.settings.timezone = "Asia/Kolkata".to_owned();
    clocks.settings.reset_time_of_day = NaiveTime::from_hms_opt(6, 0, 0).unwrap();
    let mut shifted = updated(&request);
    set_budget_reset_at(&request.fields, &mut shifted, clocks.key()).unwrap();
    assert_eq!(
        shifted.get("budget_reset_at"),
        Some(&JsonData::String("2026-05-02T00:30:00Z".to_owned()))
    );
}

#[test]
fn windows_dictionary_and_model_iso_json() {
    let request = update_request(
        r#"{"team_id":"t","budget_limits":[{"budget_duration":"1d","max_budget":10.0},{"budget_duration":"1h","max_budget":2.5,"reset_at":null}]}"#,
    );
    let mut updated_kv = updated(&request);
    set_budget_reset_at(
        &request.fields,
        &mut updated_kv,
        Clocks::at("2026-05-01T10:00:00.123456Z").key(),
    )
    .unwrap();
    let JsonData::String(text) = &updated_kv["budget_limits"] else {
        panic!("the windows are stored as serialized JSON");
    };
    assert_eq!(
        text,
        r#"[{"budget_duration":"1d","max_budget":10.0,"reset_at":"2026-05-02T00:00:00+00:00"},{"budget_duration":"1h","max_budget":2.5,"reset_at":"2026-05-01T11:00:00+00:00"}]"#
    );
}

#[rstest]
#[case::null(r#"{"team_id":"t","budget_limits":null}"#, "null")]
#[case::empty(r#"{"team_id":"t","budget_limits":[]}"#, "[]")]
fn windows_null_and_empty_untouched(#[case] document: &str, #[case] preserved: &str) {
    let request = update_request(document);
    let mut updated_kv = updated(&request);
    let before = updated_kv.clone();
    set_budget_reset_at(
        &request.fields,
        &mut updated_kv,
        Clocks::at("2026-05-01T10:00:00Z").key(),
    )
    .unwrap();
    assert_eq!(updated_kv, before);
    assert_eq!(updated_kv.get("budget_limits"), Some(&json(preserved)));
}

#[test]
fn duration_overflow_propagates() {
    let request = update_request(r#"{"team_id":"t","budget_duration":"99999999999999999999d"}"#);
    let mut updated_kv = updated(&request);
    let before = updated_kv.clone();
    let error = set_budget_reset_at(
        &request.fields,
        &mut updated_kv,
        Clocks::at("2026-05-01T10:00:00Z").key(),
    )
    .unwrap_err();
    assert!(
        matches!(&error, BudgetResetError::Duration { spec, .. } if spec == "99999999999999999999d"),
        "{error:?}"
    );
    assert_eq!(updated_kv, before);
}

#[test]
fn window_failure_keeps_supplied_limits() {
    let request = update_request(
        r#"{"team_id":"t","budget_duration":"1d","budget_limits":[{"budget_duration":"99999999999999999999d","max_budget":5.0}]}"#,
    );
    let mut updated_kv = updated(&request);
    let error = set_budget_reset_at(
        &request.fields,
        &mut updated_kv,
        Clocks::at("2026-05-01T10:00:00Z").key(),
    )
    .unwrap_err();
    assert!(matches!(error, BudgetResetError::Window(_)), "{error:?}");
    assert_eq!(
        updated_kv.get("budget_reset_at"),
        Some(&JsonData::String("2026-05-02T00:00:00Z".to_owned()))
    );
    assert!(matches!(
        updated_kv.get("budget_limits"),
        Some(JsonData::Array(_))
    ));
}

#[tokio::test]
async fn permission_created() {
    let database = TestDatabase::start().await;
    let mut data = payload(r#"{"team_id":"t","object_permission":{"mcp_servers":["srv-a"]}}"#);
    update_object_permission(&mut data, None, Some(database.prisma()), &[])
        .await
        .unwrap();
    let created = data.get("object_permission_id").cloned().unwrap();
    let JsonData::String(created_id) = created else {
        panic!("the created id is a string");
    };
    assert_eq!(data.len(), 2);
    assert_eq!(data.get("team_id"), Some(&JsonData::String("t".to_owned())));
    assert!(!data.contains_key("object_permission"));
    let row = database.permission_row(&created_id).await.unwrap();
    assert_eq!(
        row.get("mcp_servers"),
        Some(&JsonData::Array(vec![JsonData::String("srv-a".to_owned())]))
    );
    assert_eq!(database.permission_count().await, 1);
    database.close().await;
}

#[tokio::test]
async fn permission_updated() {
    let database = TestDatabase::start().await;
    database
        .seed_permission("perm-1", r#"["srv-a"]"#, None)
        .await;
    let mut data = payload(r#"{"team_id":"t","object_permission":{"models":["gpt-4o"]}}"#);
    update_object_permission(&mut data, Some("perm-1"), Some(database.prisma()), &[])
        .await
        .unwrap();
    assert_eq!(
        data.get("object_permission_id"),
        Some(&JsonData::String("perm-1".to_owned()))
    );
    let row = database.permission_row("perm-1").await.unwrap();
    assert_eq!(
        row.get("mcp_servers"),
        Some(&JsonData::Array(vec![JsonData::String("srv-a".to_owned())]))
    );
    assert_eq!(
        row.get("models"),
        Some(&JsonData::Array(vec![JsonData::String(
            "gpt-4o".to_owned()
        )]))
    );
    assert_eq!(database.permission_count().await, 1);
    database.close().await;
}

#[tokio::test]
async fn permission_without_client_errors() {
    let mut data = payload(r#"{"team_id":"t","object_permission":{"mcp_servers":["srv-a"]}}"#);
    let error = update_object_permission(&mut data, None, None, &[])
        .await
        .unwrap_err();
    assert!(
        matches!(error, ObjectPermissionError::PrismaClientMissing),
        "{error:?}"
    );
    assert!(data.contains_key("object_permission"));
    assert!(!data.contains_key("object_permission_id"));
}

#[tokio::test]
async fn permission_none_preserves_payload() {
    let database = TestDatabase::start().await;
    let mut data = payload(r#"{"team_id":"t","team_alias":"kept"}"#);
    let before = data.clone();
    update_object_permission(&mut data, None, Some(database.prisma()), &[])
        .await
        .unwrap();
    assert_eq!(data, before);

    // an explicit null grant is popped like LiteLLM's dict.pop, and no id takes its place
    let mut cleared = payload(r#"{"team_id":"t","object_permission":null}"#);
    update_object_permission(&mut cleared, None, Some(database.prisma()), &[])
        .await
        .unwrap();
    assert!(!cleared.contains_key("object_permission"));
    assert!(!cleared.contains_key("object_permission_id"));
    assert_eq!(database.permission_count().await, 0);
    database.close().await;
}

#[tokio::test]
async fn permission_failure_preserves_prior_effects() {
    let database = TestDatabase::start().await;
    let mut created = payload(r#"{"team_id":"t","object_permission":{"mcp_servers":["srv-a"]}}"#);
    update_object_permission(&mut created, None, Some(database.prisma()), &[])
        .await
        .unwrap();
    let JsonData::String(created_id) = created.get("object_permission_id").cloned().unwrap() else {
        panic!("the created id is a string");
    };
    assert_eq!(database.permission_count().await, 1);

    // the next write fails after the grant was popped from the payload, and the row written first stays
    let mut failing = payload(r#"{"team_id":"t","object_permission":"not a permission"}"#);
    let error = update_object_permission(
        &mut failing,
        Some(created_id.as_str()),
        Some(database.prisma()),
        &[],
    )
    .await
    .unwrap_err();
    assert!(
        matches!(error, ObjectPermissionError::Document(_)),
        "{error:?}"
    );
    assert!(!failing.contains_key("object_permission"));
    assert!(!failing.contains_key("object_permission_id"));
    assert_eq!(database.permission_count().await, 1);
    let row = database.permission_row(&created_id).await.unwrap();
    assert_eq!(
        row.get("mcp_servers"),
        Some(&JsonData::Array(vec![JsonData::String("srv-a".to_owned())]))
    );
    database.close().await;
}
