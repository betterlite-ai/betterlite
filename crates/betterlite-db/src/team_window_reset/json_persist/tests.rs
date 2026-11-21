#![cfg(test)]

use std::collections::BTreeMap;

use rust_decimal::Decimal;

use crate::budget_permission_rows::JsonData;
use crate::database_faults::ErrorType;
use crate::postgres_store::session::PostgresStore;
use crate::team_window_reset::json_persist::{
    WindowJsonError, decode_budget_limits, encode_budget_limits, persist_budget_limits,
    set_window_reset_at, window_fields,
};
use crate::team_window_reset::source::WindowSource;
use crate::team_window_reset::tests::support::{
    CountingReconnect, KEY_COLUMN, KEY_TABLE, budget_limits_type, close_store, decimal, finish,
    schema, serial_pool, window_field, window_json, windows_text,
};

const PAST_TEXT: &str = "2025-12-30T00:00:00+00:00";

fn object(entries: Vec<(&str, JsonData)>) -> JsonData {
    let mut fields = BTreeMap::new();
    for (name, value) in entries {
        fields.insert(name.to_owned(), value);
    }
    JsonData::Object(fields)
}

fn max_budget_of(window: &JsonData) -> Option<Decimal> {
    window_fields("k1", window).unwrap().max_budget
}

#[test]
fn decode_reads_a_list_of_windows() {
    let windows = vec![window_json("1h", Some("10"), Some(PAST_TEXT))];
    let text = serde_json::to_string(&windows).unwrap();
    assert_eq!(decode_budget_limits("k1", &text).unwrap(), windows);
}

#[test]
fn decode_reads_the_json_string_its_own_write_leaves() {
    let windows = vec![window_json("1h", Some("10"), Some(PAST_TEXT))];
    let inner = serde_json::to_string(&windows).unwrap();
    let stored = serde_json::to_string(&inner).unwrap();
    assert_eq!(decode_budget_limits("k1", &stored).unwrap(), windows);
}

#[test]
fn decode_treats_null_and_empty_text_as_no_windows() {
    assert!(decode_budget_limits("k1", "null").unwrap().is_empty());
    assert!(decode_budget_limits("k1", "\"\"").unwrap().is_empty());
    assert!(decode_budget_limits("k1", "[]").unwrap().is_empty());
}

#[test]
fn decode_rejects_a_value_that_is_not_a_list() {
    let error = decode_budget_limits("k1", "{\"budget_duration\":\"1h\"}").unwrap_err();
    assert!(matches!(error, WindowJsonError::NotAList { .. }));
}

#[test]
fn decode_rejects_a_string_that_does_not_hold_a_list() {
    let error = decode_budget_limits("k1", "\"null\"").unwrap_err();
    assert!(matches!(error, WindowJsonError::NotAList { .. }));
}

#[test]
fn decode_rejects_unreadable_json() {
    let error = decode_budget_limits("k1", "not json").unwrap_err();
    assert!(matches!(error, WindowJsonError::Unreadable { .. }));
}

#[test]
fn fields_require_a_string_budget_duration() {
    let missing = object(vec![("reset_at", JsonData::String(PAST_TEXT.to_owned()))]);
    assert!(matches!(
        window_fields("k1", &missing).unwrap_err(),
        WindowJsonError::MissingDuration { .. }
    ));
    let null = object(vec![("budget_duration", JsonData::Null)]);
    assert!(matches!(
        window_fields("k1", &null).unwrap_err(),
        WindowJsonError::MissingDuration { .. }
    ));
    let number = object(vec![(
        "budget_duration",
        JsonData::Number(serde_json::Number::from(1_u32)),
    )]);
    assert!(matches!(
        window_fields("k1", &number).unwrap_err(),
        WindowJsonError::DurationNotString { .. }
    ));
}

#[test]
fn fields_require_the_window_to_be_an_object() {
    let error = window_fields("k1", &JsonData::Array(Vec::new())).unwrap_err();
    assert!(matches!(error, WindowJsonError::NotAWindow { .. }));
}

#[test]
fn fields_read_a_numeric_max_budget() {
    let numeric = window_json("1h", Some("12.5"), Some(PAST_TEXT));
    assert_eq!(max_budget_of(&numeric), Some(decimal("12.5")));
    let integer = object(vec![
        ("budget_duration", JsonData::String("1h".to_owned())),
        (
            "max_budget",
            JsonData::Number(serde_json::Number::from(3_u32)),
        ),
    ]);
    assert_eq!(max_budget_of(&integer), Some(decimal("3")));
}

#[test]
fn fields_treat_a_missing_or_null_max_budget_as_no_cap() {
    let missing = object(vec![("budget_duration", JsonData::String("1h".to_owned()))]);
    assert_eq!(max_budget_of(&missing), None);
    let null = object(vec![
        ("budget_duration", JsonData::String("1h".to_owned())),
        ("max_budget", JsonData::Null),
    ]);
    assert_eq!(max_budget_of(&null), None);
}

#[test]
fn fields_reject_a_max_budget_that_is_not_a_number() {
    let text = object(vec![
        ("budget_duration", JsonData::String("1h".to_owned())),
        ("max_budget", JsonData::String("12.5".to_owned())),
    ]);
    assert!(matches!(
        window_fields("k1", &text).unwrap_err(),
        WindowJsonError::MaxBudgetNotNumber { .. }
    ));
    let boolean = object(vec![
        ("budget_duration", JsonData::String("1h".to_owned())),
        ("max_budget", JsonData::Bool(true)),
    ]);
    assert!(matches!(
        window_fields("k1", &boolean).unwrap_err(),
        WindowJsonError::MaxBudgetNotNumber { .. }
    ));
    let list = object(vec![
        ("budget_duration", JsonData::String("1h".to_owned())),
        ("max_budget", JsonData::Array(Vec::new())),
    ]);
    assert!(matches!(
        window_fields("k1", &list).unwrap_err(),
        WindowJsonError::MaxBudgetNotNumber { .. }
    ));
}

#[test]
fn fields_treat_a_null_or_absent_reset_at_as_none() {
    let window = window_json("1h", None, None);
    assert_eq!(window_fields("k1", &window).unwrap().reset_at, None);
    let null = object(vec![
        ("budget_duration", JsonData::String("1h".to_owned())),
        ("reset_at", JsonData::Null),
    ]);
    assert_eq!(window_fields("k1", &null).unwrap().reset_at, None);
    let text = window_json("1h", None, Some(PAST_TEXT));
    assert_eq!(
        window_fields("k1", &text).unwrap().reset_at,
        Some(PAST_TEXT)
    );
}

#[test]
fn encode_preserves_fields_the_reset_does_not_know() {
    let window = object(vec![
        ("budget_duration", JsonData::String("1h".to_owned())),
        ("custom_flag", JsonData::Bool(true)),
        ("reset_at", JsonData::String(PAST_TEXT.to_owned())),
    ]);
    let text = encode_budget_limits("k1", &[window]).unwrap();
    let decoded = decode_budget_limits("k1", &text).unwrap();
    let first = decoded.first().unwrap();
    assert_eq!(
        window_field(first, "custom_flag"),
        Some(&JsonData::Bool(true))
    );
}

#[test]
fn set_reset_at_replaces_only_that_field() {
    let mut window = window_json("1h", Some("10"), Some(PAST_TEXT));
    set_window_reset_at("k1", &mut window, "2026-01-01T01:00:00+00:00".to_owned()).unwrap();
    let fields = window_fields("k1", &window).unwrap();
    assert_eq!(fields.reset_at, Some("2026-01-01T01:00:00+00:00"));
    assert_eq!(fields.budget_duration, "1h");
    assert_eq!(fields.max_budget, Some(decimal("10")));
}

#[tokio::test]
async fn persist_stores_the_list_as_a_json_string() {
    let (fixture, store, client) = schema().await;
    client
        .batch_execute("INSERT INTO \"LiteLLM_VerificationToken\" (token) VALUES ('k1')")
        .await
        .unwrap();
    let windows = vec![window_json("1h", Some("10"), Some(PAST_TEXT))];
    let payload = encode_budget_limits("k1", &windows).unwrap();
    persist_budget_limits(
        &store,
        WindowSource::Keys,
        "k1",
        &payload,
        None::<&mut CountingReconnect>,
    )
    .await
    .unwrap();
    assert_eq!(
        budget_limits_type(&client, KEY_TABLE, KEY_COLUMN, "k1").await,
        "string"
    );
    assert_eq!(
        windows_text(&client, KEY_TABLE, KEY_COLUMN, "k1").await,
        windows
    );
    finish(fixture, store, client).await;
}

#[tokio::test]
async fn persist_reports_a_row_that_no_longer_exists() {
    let (fixture, store, client) = schema().await;
    let error = persist_budget_limits(
        &store,
        WindowSource::Teams,
        "missing",
        "[]",
        None::<&mut CountingReconnect>,
    )
    .await
    .unwrap_err();
    assert_eq!(error.error_type, ErrorType::RecordNotFound);
    assert!(error.message.contains("team_id = missing"));
    finish(fixture, store, client).await;
}

#[tokio::test]
async fn persist_surfaces_a_rejected_update_without_retrying() {
    let (fixture, store, client) = schema().await;
    client
        .batch_execute(
            "INSERT INTO \"LiteLLM_VerificationToken\" (token) VALUES ('k1');
             CREATE FUNCTION reject_budget_limits() RETURNS trigger AS $$
             BEGIN RAISE EXCEPTION 'budget limits are frozen'; END; $$ LANGUAGE plpgsql;
             CREATE TRIGGER reject_budget_limits BEFORE UPDATE ON \"LiteLLM_VerificationToken\"
             FOR EACH ROW EXECUTE FUNCTION reject_budget_limits();",
        )
        .await
        .unwrap();
    let mut reconnect = CountingReconnect::new(true);
    let error = persist_budget_limits(&store, WindowSource::Keys, "k1", "[]", Some(&mut reconnect))
        .await
        .unwrap_err();
    assert_eq!(error.error_type, ErrorType::Other);
    assert_eq!(reconnect.attempts(), 0_usize);
    finish(fixture, store, client).await;
}

#[tokio::test]
async fn persist_retries_a_pool_timeout_after_reconnecting() {
    let (fixture, store, client) = schema().await;
    client
        .batch_execute("INSERT INTO \"LiteLLM_VerificationToken\" (token) VALUES ('k1')")
        .await
        .unwrap();
    let serial = PostgresStore {
        pool: Some(serial_pool(&fixture.options.url)),
    };
    let held = serial.session().await.unwrap();
    let mut reconnect = CountingReconnect::releasing(held);
    persist_budget_limits(
        &serial,
        WindowSource::Keys,
        "k1",
        "[]",
        Some(&mut reconnect),
    )
    .await
    .unwrap();
    assert_eq!(reconnect.attempts(), 1_usize);
    assert_eq!(
        reconnect.reasons(),
        vec!["reset_budget_write_key_windows_failure".to_owned()]
    );
    close_store(serial);
    finish(fixture, store, client).await;
}
