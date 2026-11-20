#![cfg(test)]

mod support;

use betterlite_models::decisions::Json;
use chrono::{DateTime, TimeZone as _, Utc};
use rstest::rstest;
use support::{Seed, capture_logs, insert_seed, unconnected};

use crate::health_check_rows::{HealthCheckRow, HealthCheckRowsError, NewHealthCheck};

const NO_DATABASE: &str = "No DB Connected. See - https://docs.litellm.ai/docs/proxy/virtual_keys";

fn instant(text: &str) -> DateTime<Utc> {
    text.parse().unwrap()
}

fn check(model_name: &str) -> NewHealthCheck {
    NewHealthCheck {
        model_name: model_name.to_owned(),
        model_id: Some(String::from("dep-1")),
        status: String::from("healthy"),
        healthy_count: 2,
        unhealthy_count: 1,
        error_message: Some(String::from("boom")),
        response_time_ms: Some(12.5_f64),
        checked_by: Some(String::from("user-1")),
    }
}

#[tokio::test]
async fn latest_returns_the_newest_row_per_pair_in_sql_order() {
    let database = support::start().await;
    insert_seed(
        &database.client,
        &Seed {
            status: "healthy",
            model_id: Some("dep-1"),
            ..Seed::new("old", "gpt-4o", "2026-01-01 10:00:00")
        },
    )
    .await;
    insert_seed(
        &database.client,
        &Seed {
            status: "unhealthy",
            model_id: Some("dep-1"),
            ..Seed::new("new", "gpt-4o", "2026-01-01 11:00:00")
        },
    )
    .await;
    insert_seed(
        &database.client,
        &Seed::new("none", "gpt-4o", "2026-01-01 09:00:00"),
    )
    .await;
    let rows = database.rows().fetch_latest().await;
    let ids: Vec<&str> = rows
        .iter()
        .map(|row| row.health_check_id.as_str())
        .collect();
    assert_eq!(ids, ["new", "none"]);
    assert_eq!(rows[0].status, "unhealthy");
    database.finish().await;
}

#[tokio::test]
async fn latest_keeps_null_model_id_pairs_apart() {
    let database = support::start().await;
    insert_seed(
        &database.client,
        &Seed::new("a", "gpt-4o", "2026-01-01 10:00:00"),
    )
    .await;
    insert_seed(
        &database.client,
        &Seed::new("b", "gpt-4o", "2026-01-01 11:00:00"),
    )
    .await;
    insert_seed(
        &database.client,
        &Seed::new("c", "claude", "2026-01-01 10:00:00"),
    )
    .await;
    let rows = database.rows().fetch_latest().await;
    let ids: Vec<&str> = rows
        .iter()
        .map(|row| row.health_check_id.as_str())
        .collect();
    assert_eq!(ids, ["c", "b"]);
    assert!(rows.iter().all(|row| row.model_id.is_none()));
    database.finish().await;
}

#[tokio::test]
async fn latest_reads_naive_timestamps_as_utc() {
    let database = support::start().await;
    insert_seed(
        &database.client,
        &Seed::new("a", "gpt-4o", "2026-01-01 10:00:00.123"),
    )
    .await;
    let rows = database.rows().fetch_latest().await;
    assert_eq!(rows[0].checked_at, instant("2026-01-01T10:00:00.123Z"));
    assert_eq!(rows[0].created_at, instant("2026-01-01T10:00:00.123Z"));
    assert_eq!(rows[0].updated_at, instant("2026-01-01T10:00:00.123Z"));
    database.finish().await;
}

#[tokio::test]
async fn latest_decodes_details_json_text() {
    let database = support::start().await;
    insert_seed(
        &database.client,
        &Seed {
            details: Some(r#""{\"inner\": 1}""#),
            ..Seed::new("a", "gpt-4o", "2026-01-01 10:00:00")
        },
    )
    .await;
    let rows = database.rows().fetch_latest().await;
    let expected: Json = serde_json::from_str(r#"{"inner": 1}"#).unwrap();
    assert_eq!(rows[0].details, Some(expected));
    database.finish().await;
}

#[tokio::test]
async fn latest_fails_the_whole_read_when_details_text_is_not_json() {
    let database = support::start().await;
    insert_seed(
        &database.client,
        &Seed::new("a", "gpt-4o", "2026-01-01 10:00:00"),
    )
    .await;
    insert_seed(
        &database.client,
        &Seed {
            details: Some(r#""not json""#),
            ..Seed::new("b", "gpt-4o-mini", "2026-01-01 10:00:00")
        },
    )
    .await;
    let rows = database.rows();
    assert!(matches!(
        rows.query_latest().await,
        Err(HealthCheckRowsError::Details { .. })
    ));
    let (logs, _guard) = capture_logs();
    assert!(rows.fetch_latest().await.is_empty());
    assert!(logs.text().contains(
        "Error getting all latest health checks: the details column holds text that is not JSON"
    ));
    database.finish().await;
}

#[tokio::test]
async fn latest_logs_the_error_and_returns_empty() {
    let (logs, _guard) = capture_logs();
    let rows = unconnected().fetch_latest().await;
    assert!(rows.is_empty());
    assert!(logs.text().contains(&format!(
        "Error getting all latest health checks: {NO_DATABASE}"
    )));
}

#[tokio::test]
async fn models_filters_by_name() {
    let database = support::start().await;
    insert_seed(
        &database.client,
        &Seed::new("a", "gpt-4o", "2026-01-01 10:00:00"),
    )
    .await;
    insert_seed(
        &database.client,
        &Seed::new("b", "claude", "2026-01-01 10:00:00"),
    )
    .await;
    let names = vec![String::from("gpt-4o")];
    let rows = database.rows().fetch_latest_for_models(&names).await;
    let ids: Vec<&str> = rows
        .iter()
        .map(|row| row.health_check_id.as_str())
        .collect();
    assert_eq!(ids, ["a"]);
    database.finish().await;
}

#[tokio::test]
async fn models_returns_empty_for_no_names() {
    let (logs, _guard) = capture_logs();
    let rows = unconnected().fetch_latest_for_models(&[]).await;
    assert!(rows.is_empty());
    assert!(logs.text().is_empty());
}

#[tokio::test]
async fn models_logs_the_error_and_returns_empty() {
    let (logs, _guard) = capture_logs();
    let names = vec![String::from("gpt-4o")];
    let rows = unconnected().fetch_latest_for_models(&names).await;
    assert!(rows.is_empty());
    assert!(logs.text().contains(&format!(
        "Error getting latest health checks for models: {NO_DATABASE}"
    )));
}

#[tokio::test]
async fn save_writes_the_row_and_returns_it() {
    let database = support::start().await;
    let row = database
        .rows_with_id("fixed-id")
        .save_result(check("gpt-4o"))
        .await
        .unwrap();
    assert_eq!(row.health_check_id, "fixed-id");
    assert_eq!(row.model_name, "gpt-4o");
    assert_eq!(row.model_id.as_deref(), Some("dep-1"));
    assert_eq!(row.status, "healthy");
    assert_eq!(row.healthy_count, 2);
    assert_eq!(row.unhealthy_count, 1);
    assert_eq!(row.error_message.as_deref(), Some("boom"));
    assert_eq!(row.response_time_ms, Some(12.5_f64));
    assert_eq!(row.checked_by.as_deref(), Some("user-1"));
    assert!(row.details.is_none());
    assert!(row.checked_at > instant("2025-01-01T00:00:00Z"));
    assert_eq!(row.updated_at, row.created_at);
    database.finish().await;
}

#[tokio::test]
async fn save_truncates_the_error_to_500_chars() {
    let database = support::start().await;
    let long = "x".repeat(600);
    let row = database
        .rows()
        .save_result(NewHealthCheck {
            error_message: Some(long),
            ..check("gpt-4o")
        })
        .await
        .unwrap();
    let stored = row.error_message.unwrap();
    assert_eq!(stored.len(), 500);
    assert_eq!(stored, "x".repeat(500));
    database.finish().await;
}

#[tokio::test]
async fn save_truncates_the_error_by_characters() {
    let database = support::start().await;
    let long = "é".repeat(600);
    let row = database
        .rows()
        .save_result(NewHealthCheck {
            error_message: Some(long),
            ..check("gpt-4o")
        })
        .await
        .unwrap();
    let stored = row.error_message.unwrap();
    assert_eq!(stored.chars().count(), 500);
    assert_eq!(stored.len(), 1000);
    database.finish().await;
}

#[tokio::test]
async fn save_stores_empty_optional_fields_as_null() {
    let database = support::start().await;
    let row = database
        .rows()
        .save_result(NewHealthCheck {
            model_id: Some(String::new()),
            checked_by: Some(String::new()),
            error_message: Some(String::new()),
            ..check("gpt-4o")
        })
        .await
        .unwrap();
    assert!(row.model_id.is_none());
    assert!(row.checked_by.is_none());
    assert!(row.error_message.is_none());
    database.finish().await;
}

#[rstest]
#[case::nan(f64::NAN)]
#[case::infinity(f64::INFINITY)]
#[case::negative_infinity(f64::NEG_INFINITY)]
#[tokio::test]
async fn save_stores_a_non_finite_time_as_null(#[case] value: f64) {
    let database = support::start().await;
    let row = database
        .rows()
        .save_result(NewHealthCheck {
            response_time_ms: Some(value),
            ..check("gpt-4o")
        })
        .await
        .unwrap();
    assert!(row.response_time_ms.is_none());
    database.finish().await;
}

#[tokio::test]
async fn save_returns_none_and_logs_when_the_insert_fails() {
    let (logs, _guard) = capture_logs();
    let row = unconnected().save_result(check("gpt-4o")).await;
    assert!(row.is_none());
    assert!(logs.text().contains(&format!(
        "Error saving health check result for model gpt-4o: {NO_DATABASE}"
    )));
}

#[tokio::test]
async fn history_filters_by_non_empty_model_name_and_status() {
    let database = support::start().await;
    insert_seed(
        &database.client,
        &Seed::new("a", "gpt-4o", "2026-01-01 10:00:00"),
    )
    .await;
    insert_seed(
        &database.client,
        &Seed {
            status: "unhealthy",
            ..Seed::new("b", "gpt-4o", "2026-01-01 11:00:00")
        },
    )
    .await;
    insert_seed(
        &database.client,
        &Seed {
            status: "unhealthy",
            ..Seed::new("c", "claude", "2026-01-01 12:00:00")
        },
    )
    .await;
    let rows = database
        .rows()
        .history(Some("gpt-4o"), Some("unhealthy"), 100, 0)
        .await;
    let ids: Vec<&str> = rows
        .iter()
        .map(|row| row.health_check_id.as_str())
        .collect();
    assert_eq!(ids, ["b"]);
    database.finish().await;
}

#[tokio::test]
async fn history_drops_empty_filters() {
    let database = support::start().await;
    insert_seed(
        &database.client,
        &Seed::new("a", "gpt-4o", "2026-01-01 10:00:00"),
    )
    .await;
    insert_seed(
        &database.client,
        &Seed {
            status: "unhealthy",
            ..Seed::new("b", "claude", "2026-01-01 11:00:00")
        },
    )
    .await;
    let rows = database.rows().history(Some(""), Some(""), 100, 0).await;
    assert_eq!(rows.len(), 2);
    database.finish().await;
}

#[tokio::test]
async fn history_orders_newest_first() {
    let database = support::start().await;
    insert_seed(
        &database.client,
        &Seed::new("a", "gpt-4o", "2026-01-01 10:00:00"),
    )
    .await;
    insert_seed(
        &database.client,
        &Seed::new("b", "gpt-4o", "2026-01-01 12:00:00"),
    )
    .await;
    insert_seed(
        &database.client,
        &Seed::new("c", "gpt-4o", "2026-01-01 11:00:00"),
    )
    .await;
    let rows = database.rows().history(None, None, 100, 0).await;
    let ids: Vec<&str> = rows
        .iter()
        .map(|row| row.health_check_id.as_str())
        .collect();
    assert_eq!(ids, ["b", "c", "a"]);
    database.finish().await;
}

#[tokio::test]
async fn history_honours_limit_and_offset() {
    let database = support::start().await;
    insert_seed(
        &database.client,
        &Seed::new("a", "gpt-4o", "2026-01-01 10:00:00"),
    )
    .await;
    insert_seed(
        &database.client,
        &Seed::new("b", "gpt-4o", "2026-01-01 12:00:00"),
    )
    .await;
    insert_seed(
        &database.client,
        &Seed::new("c", "gpt-4o", "2026-01-01 11:00:00"),
    )
    .await;
    let rows = database.rows().history(None, None, 1, 1).await;
    let ids: Vec<&str> = rows
        .iter()
        .map(|row| row.health_check_id.as_str())
        .collect();
    assert_eq!(ids, ["c"]);
    database.finish().await;
}

#[tokio::test]
async fn history_logs_the_error_and_returns_empty() {
    let (logs, _guard) = capture_logs();
    let rows = unconnected().history(None, None, 10, 0).await;
    assert!(rows.is_empty());
    assert!(logs.text().contains(&format!(
        "Error getting health check history: {NO_DATABASE}"
    )));
}

#[tokio::test]
async fn history_reads_naive_timestamps_as_utc() {
    let database = support::start().await;
    insert_seed(
        &database.client,
        &Seed::new("a", "gpt-4o", "2026-01-01 10:00:00"),
    )
    .await;
    let rows: Vec<HealthCheckRow> = database.rows().history(None, None, 100, 0).await;
    assert_eq!(
        rows[0].checked_at,
        Utc.with_ymd_and_hms(2026, 1, 1, 10, 0, 0).unwrap()
    );
    database.finish().await;
}
