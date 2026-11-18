#![cfg(test)]

use std::fs;
use std::path::Path;
use std::sync::Arc;

use axum::http::StatusCode;
use betterlite_db::budget_permission_rows::JsonData;
use rstest::rstest;

use crate::roi_settings::route::parse_update_body;
use crate::roi_settings::support::MemoryRepository;
use crate::roi_settings::support::admin_caller;
use crate::roi_settings::support::call;
use crate::roi_settings::support::catalog;
use crate::roi_settings::support::erased;
use crate::roi_settings::support::loc_names;
use crate::roi_settings::support::put_request;
use crate::roi_settings::support::settings_app;

#[rstest]
#[case::backfill_days_below_the_range(
    r#"{"backfill_days":0}"#,
    "greater_than_equal",
    &["body", "backfill_days"],
    "greater than or equal to 1"
)]
#[case::backfill_days_above_the_range(
    r#"{"backfill_days":3651}"#,
    "less_than_equal",
    &["body", "backfill_days"],
    "less than or equal to 3650"
)]
#[case::interval_above_the_range(
    r#"{"update_interval_minutes":43201}"#,
    "less_than_equal",
    &["body", "update_interval_minutes"],
    "less than or equal to 43200"
)]
#[case::interval_below_the_range(
    r#"{"update_interval_minutes":-1}"#,
    "greater_than_equal",
    &["body", "update_interval_minutes"],
    "greater than or equal to 0"
)]
#[case::interval_below_five_minutes(
    r#"{"update_interval_minutes":3}"#,
    "value_error",
    &["update_interval_minutes"],
    "Choose manual updates (0), or an interval of at least 5 minutes."
)]
#[case::insecure_url(
    r#"{"github_api_url":"http://github.example.com"}"#,
    "value_error",
    &["github_api_url"],
    "Use an HTTPS GitHub API URL"
)]
#[case::empty_url(
    r#"{"github_api_url":""}"#,
    "value_error",
    &["github_api_url"],
    "A GitHub API URL is required."
)]
#[case::invalid_repository(
    r#"{"repos":["not-a-repository"]}"#,
    "value_error",
    &["repos"],
    "Repositories must use owner/repo format."
)]
#[case::empty_prompt(
    r#"{"estimator_prompt":"   "}"#,
    "value_error",
    &["estimator_prompt"],
    "between 1 and 20,000 characters"
)]
#[case::unknown_field(
    r#"{"unknown":1}"#,
    "extra_forbidden",
    &["body", "unknown"],
    "Extra inputs are not permitted"
)]
#[case::wrong_field_type(
    r#"{"backfill_days":"soon"}"#,
    "value_error",
    &["body", "backfill_days"],
    "invalid type"
)]
#[case::malformed_json(
    r#"{"backfill_days":"#,
    "json_invalid",
    &["body"],
    "JSON decode error"
)]
#[case::nan_literal(
    r#"{"update_interval_minutes":NaN}"#,
    "json_invalid",
    &["body"],
    "JSON decode error"
)]
#[case::infinity_literal(
    r#"{"update_interval_minutes":Infinity}"#,
    "json_invalid",
    &["body"],
    "JSON decode error"
)]
#[case::empty_body("", "missing", &["body"], "Field required")]
#[case::null_body("null", "missing", &["body"], "Field required")]
#[case::array_body(
    "[]",
    "model_attributes_type",
    &["body"],
    "Input should be a valid dictionary or object to extract fields from"
)]
#[tokio::test]
async fn put_validation_rejects_an_invalid_patch(
    #[case] body: &str,
    #[case] kind: &str,
    #[case] loc: &[&str],
    #[case] message: &str,
) {
    let memory = Arc::new(MemoryRepository::default());
    let app = settings_app(Some(erased(&memory)), Some(catalog(Vec::new())));
    let reply = call(app, put_request(Some(&admin_caller()), body)).await;
    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
    let issues = reply.issues();
    assert_eq!(issues.len(), 1);
    let issue = &issues[0];
    assert_eq!(issue.kind, kind);
    let expected: Vec<String> = loc.iter().map(|name| (*name).to_owned()).collect();
    assert_eq!(loc_names(issue), expected);
    assert!(issue.msg.contains(message), "{}", issue.msg);
    assert!(memory.is_empty());
}

#[rstest]
#[case::request_bound(r#"{"backfill_days":0}"#, "Input should be greater than or equal to 1")]
#[case::settings_value(
    r#"{"update_interval_minutes":3}"#,
    "Value error, Choose manual updates (0), or an interval of at least 5 minutes."
)]
#[tokio::test]
async fn put_validation_messages_carry_pydantic_text(#[case] body: &str, #[case] message: &str) {
    let memory = Arc::new(MemoryRepository::default());
    let app = settings_app(Some(erased(&memory)), Some(catalog(Vec::new())));
    let reply = call(app, put_request(Some(&admin_caller()), body)).await;
    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
    let issues = reply.issues();
    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].msg, message);
}

#[rstest]
#[case::manual_updates(0.0_f64)]
#[case::five_minutes(5.0_f64)]
#[case::one_day(1_440.0_f64)]
#[tokio::test]
async fn put_validation_accepts_a_valid_interval(#[case] minutes: f64) {
    let memory = Arc::new(MemoryRepository::default());
    let app = settings_app(Some(erased(&memory)), Some(catalog(Vec::new())));
    let reply = call(
        app,
        put_request(
            Some(&admin_caller()),
            &format!(r#"{{"update_interval_minutes":{minutes}}}"#),
        ),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    let saved = memory.stored().expect("saved settings");
    assert_eq!(saved.update_interval_minutes.to_bits(), minutes.to_bits());
}

#[rstest]
#[case::lowest_accepted(1)]
#[case::highest_accepted(3650)]
#[tokio::test]
async fn put_validation_accepts_a_valid_backfill_days(#[case] days: i64) {
    let memory = Arc::new(MemoryRepository::default());
    let app = settings_app(Some(erased(&memory)), Some(catalog(Vec::new())));
    let reply = call(
        app,
        put_request(
            Some(&admin_caller()),
            &format!(r#"{{"backfill_days":{days}}}"#),
        ),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    let saved = memory.stored().expect("saved settings");
    assert_eq!(saved.backfill_days, days);
}

#[tokio::test]
async fn put_validation_issues_carry_only_type_loc_and_msg() {
    let memory = Arc::new(MemoryRepository::default());
    let app = settings_app(Some(erased(&memory)), Some(catalog(Vec::new())));
    let reply = call(
        app,
        put_request(Some(&admin_caller()), r#"{"backfill_days":0}"#),
    )
    .await;
    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
    let JsonData::Object(root) = reply.json::<JsonData>() else {
        panic!("the error body is not an object");
    };
    let Some(JsonData::Array(issues)) = root.get("detail") else {
        panic!("the detail is not a list");
    };
    let JsonData::Object(issue) = &issues[0] else {
        panic!("the issue is not an object");
    };
    let names: Vec<&str> = issue.keys().map(String::as_str).collect();
    assert_eq!(names, ["loc", "msg", "type"]);
}

#[tokio::test]
async fn put_validation_rejects_a_duplicate_field() {
    let memory = Arc::new(MemoryRepository::default());
    let app = settings_app(Some(erased(&memory)), Some(catalog(Vec::new())));
    let reply = call(
        app,
        put_request(
            Some(&admin_caller()),
            r#"{"backfill_days":10,"backfill_days":20}"#,
        ),
    )
    .await;
    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(reply.issues().len(), 1);
    assert!(memory.is_empty());
}

#[tokio::test]
async fn put_validation_reports_the_first_bad_field_only() {
    let memory = Arc::new(MemoryRepository::default());
    let app = settings_app(Some(erased(&memory)), Some(catalog(Vec::new())));
    let reply = call(
        app,
        put_request(
            Some(&admin_caller()),
            r#"{"backfill_days":0,"update_interval_minutes":-1}"#,
        ),
    )
    .await;
    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
    let issues = reply.issues();
    assert_eq!(issues.len(), 1);
    assert_eq!(loc_names(&issues[0]), ["body", "backfill_days"]);
}

#[test]
fn replays_the_roi_settings_body_corpus() {
    assert!(replay_roi_settings_body_corpus() > 0);
}

fn replay_roi_settings_body_corpus() -> usize {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/roi_settings_body");
    let entries = fs::read_dir(&root).expect("the corpus directory");
    let mut replayed = 0_usize;
    for entry in entries {
        let bytes = fs::read(entry.expect("a corpus entry").path()).expect("a corpus file");
        drop(parse_update_body(&bytes));
        replayed = replayed.saturating_add(1);
    }
    replayed
}
