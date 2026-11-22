use rstest::rstest;

use crate::spend_counter_batch::batch::SpendCounterBatchError;
use crate::spend_increments::error::SpendIncrementError;
use crate::spend_increments::increment_spend_counters;
use crate::spend_writes::error::SpendWriteError;

use super::support::{Fixture, Logs, decimal, fixture_instant, handle, input};

#[tokio::test]
async fn a_key_window_charges_its_counter_and_queues_its_row() {
    let fixture = Fixture::new(false);
    fixture.sources.put(
        "tok",
        r#"{"budget_limits": [{"budget_duration": "1d", "reset_at": "2026-11-18T00:00:00+00:00"}]}"#,
    );
    let mut request = input(Some("tok"));
    request.request_started_at = Some(fixture_instant());

    increment_spend_counters(&request, &fixture.services())
        .await
        .unwrap();

    assert_eq!(fixture.counter("spend:key:tok").await, Some(decimal("2")));
    assert_eq!(
        fixture.counter("spend:key:tok:window:1d").await,
        Some(decimal("2"))
    );
    let records = fixture.queue.records();
    assert_eq!(records.len(), 1);
    let row = &records[0];
    assert_eq!(row.entity_type, "key");
    assert_eq!(row.entity_id, "tok");
    assert_eq!(row.window_duration, "1d");
    assert_eq!(row.window_start, "2026-11-17T00:00:00.000000");
    assert_eq!(row.spend.0, decimal("2"));
    assert_eq!(
        row.started_at.as_deref(),
        Some("2026-11-18T12:00:00.000000")
    );
}

#[tokio::test]
async fn a_team_window_charges_its_counter_and_queues_a_team_row() {
    let fixture = Fixture::new(false);
    fixture.sources.put(
        "team_id:t1",
        r#"{"budget_limits": [{"budget_duration": "1h", "reset_at": "2026-11-18T00:00:00+00:00"}]}"#,
    );
    let mut request = input(None);
    request.team_id = Some("t1");

    increment_spend_counters(&request, &fixture.services())
        .await
        .unwrap();

    assert_eq!(fixture.counter("spend:team:t1").await, Some(decimal("2")));
    assert_eq!(
        fixture.counter("spend:team:t1:window:1h").await,
        Some(decimal("2"))
    );
    let records = fixture.queue.records();
    assert_eq!(records.len(), 1);
    let row = &records[0];
    assert_eq!(row.entity_type, "team");
    assert_eq!(row.entity_id, "t1");
    assert_eq!(row.window_duration, "1h");
    assert_eq!(row.window_start, "2026-11-17T23:00:00.000000");
    assert_eq!(row.started_at, None);
}

#[tokio::test]
async fn a_window_without_reset_at_charges_but_queues_no_row() {
    let fixture = Fixture::new(false);
    fixture
        .sources
        .put("tok", r#"{"budget_limits": [{"budget_duration": "1d"}]}"#);

    increment_spend_counters(&input(Some("tok")), &fixture.services())
        .await
        .unwrap();

    assert_eq!(
        fixture.counter("spend:key:tok:window:1d").await,
        Some(decimal("2"))
    );
    assert!(fixture.queue.records().is_empty());
}

#[tokio::test]
async fn an_empty_reset_at_charges_but_queues_no_row() {
    let fixture = Fixture::new(false);
    fixture.sources.put(
        "tok",
        r#"{"budget_limits": [{"budget_duration": "1d", "reset_at": ""}]}"#,
    );

    increment_spend_counters(&input(Some("tok")), &fixture.services())
        .await
        .unwrap();

    assert_eq!(
        fixture.counter("spend:key:tok:window:1d").await,
        Some(decimal("2"))
    );
    assert!(fixture.queue.records().is_empty());
}

#[tokio::test]
async fn an_invalid_reset_at_slides_to_now_but_still_queues_a_row() {
    let fixture = Fixture::new(false);
    fixture.sources.put(
        "tok",
        r#"{"budget_limits": [{"budget_duration": "1d", "reset_at": "nope"}]}"#,
    );

    increment_spend_counters(&input(Some("tok")), &fixture.services())
        .await
        .unwrap();

    assert_eq!(
        fixture.counter("spend:key:tok:window:1d").await,
        Some(decimal("2"))
    );
    let records = fixture.queue.records();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].window_start, "2026-11-17T12:00:00.000000");
}

#[tokio::test]
async fn an_unmappable_window_warns_and_charges_nothing() {
    let fixture = Fixture::new(false);
    let logs = Logs::capture();
    fixture.sources.put(
        "tok",
        r#"{"budget_limits": [{"budget_duration": "2w", "reset_at": "2026-11-18T00:00:00+00:00"}]}"#,
    );

    increment_spend_counters(&input(Some("tok")), &fixture.services())
        .await
        .unwrap();

    assert_eq!(fixture.counter("spend:key:tok").await, Some(decimal("2")));
    assert_eq!(fixture.counter("spend:key:tok:window:2w").await, None);
    assert!(fixture.queue.records().is_empty());
    assert!(logs.contains(
        "Skipping spend counter increment for invalid budget window spend:key:tok:window:2w"
    ));
}

#[tokio::test]
async fn a_cold_window_that_cannot_be_loaded_charges_nothing_but_queues_its_row() {
    let fixture = Fixture::new(false);
    let logs = Logs::capture();
    fixture.rows.fail();
    fixture.sources.put(
        "tok",
        r#"{"budget_limits": [{"budget_duration": "1d", "reset_at": "2026-11-18T00:00:00+00:00"}]}"#,
    );

    increment_spend_counters(&input(Some("tok")), &fixture.services())
        .await
        .unwrap();

    assert_eq!(fixture.counter("spend:key:tok").await, Some(decimal("2")));
    assert_eq!(fixture.counter("spend:key:tok:window:1d").await, None);
    let records = fixture.queue.records();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].entity_id, "tok");
    assert!(logs.contains("Skipping cold spend counter seed"));
}

#[tokio::test]
async fn a_bad_window_fails_alone_while_its_sibling_charges() {
    let fixture = Fixture::new(false);
    fixture.batch.fail_for("spend:key:tok:window:1d");
    fixture.sources.put(
        "tok",
        r#"{"budget_limits": [{"budget_duration": "1d", "reset_at": "2026-11-18T00:00:00+00:00"}, {"budget_duration": "1h", "reset_at": "2026-11-18T00:00:00+00:00"}]}"#,
    );

    let result = increment_spend_counters(&input(Some("tok")), &fixture.services()).await;

    assert!(matches!(
        result,
        Err(SpendIncrementError::Write(SpendWriteError::Batch(
            SpendCounterBatchError::Cancelled
        )))
    ));
    assert_eq!(fixture.counter("spend:key:tok").await, Some(decimal("2")));
    assert_eq!(fixture.counter("spend:key:tok:window:1d").await, None);
    assert_eq!(
        fixture.counter("spend:key:tok:window:1h").await,
        Some(decimal("2"))
    );
    let records = fixture.queue.records();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].window_duration, "1h");
}

#[tokio::test]
async fn a_reserved_window_counter_still_queues_its_row() {
    let fixture = Fixture::new(false);
    fixture.counters.reseed_unavailable();
    fixture.sources.put(
        "tok",
        r#"{"budget_limits": [{"budget_duration": "1d", "reset_at": "2026-11-18T00:00:00+00:00"}]}"#,
    );
    let reservation = handle("5", &["spend:key:tok:window:1d"]);
    let mut request = input(Some("tok"));
    request.budget_reservation = Some(&reservation);

    increment_spend_counters(&request, &fixture.services())
        .await
        .unwrap();

    assert_eq!(fixture.counter("spend:key:tok:window:1d").await, None);
    assert_eq!(fixture.counter("spend:key:tok").await, Some(decimal("2")));
    let records = fixture.queue.records();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].window_duration, "1d");
    assert_eq!(records[0].window_start, "2026-11-17T00:00:00.000000");
}

#[tokio::test]
async fn an_invalid_duration_is_skipped_while_its_sibling_charges() {
    let fixture = Fixture::new(false);
    let logs = Logs::capture();
    fixture.sources.put(
        "tok",
        r#"{"budget_limits": [{"reset_at": "2026-11-18T00:00:00+00:00"}, {"budget_duration": "1d", "reset_at": "2026-11-18T00:00:00+00:00"}]}"#,
    );

    increment_spend_counters(&input(Some("tok")), &fixture.services())
        .await
        .unwrap();

    assert_eq!(fixture.counter("spend:key:tok").await, Some(decimal("2")));
    assert_eq!(
        fixture.counter("spend:key:tok:window:1d").await,
        Some(decimal("2"))
    );
    let records = fixture.queue.records();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].window_duration, "1d");
    assert!(logs.contains("Skipping budget window with invalid duration"));
}

#[rstest]
#[case::list(
    r#"{"budget_limits": [{"budget_duration": "1d", "reset_at": "2026-11-18T00:00:00+00:00"}]}"#
)]
#[case::json_string(
    r#"{"budget_limits": "[{\"budget_duration\": \"1d\", \"reset_at\": \"2026-11-18T00:00:00+00:00\"}]"}"#
)]
#[case::string_entry(
    r#"{"budget_limits": ["{\"budget_duration\": \"1d\", \"reset_at\": \"2026-11-18T00:00:00+00:00\"}"]}"#
)]
#[tokio::test]
async fn a_cached_budget_limits_list_charges_its_windows(#[case] source: &str) {
    let fixture = Fixture::new(false);
    fixture.sources.put("tok", source);

    increment_spend_counters(&input(Some("tok")), &fixture.services())
        .await
        .unwrap();

    assert_eq!(
        fixture.counter("spend:key:tok:window:1d").await,
        Some(decimal("2"))
    );
    assert_eq!(fixture.queue.records().len(), 1);
}

#[tokio::test]
async fn a_queue_failure_is_logged_and_the_increment_still_applies() {
    let fixture = Fixture::new(false);
    let logs = Logs::capture();
    fixture.queue.fail();
    fixture.sources.put(
        "tok",
        r#"{"budget_limits": [{"budget_duration": "1d", "reset_at": "2026-11-18T00:00:00+00:00"}]}"#,
    );

    increment_spend_counters(&input(Some("tok")), &fixture.services())
        .await
        .unwrap();

    assert_eq!(
        fixture.counter("spend:key:tok:window:1d").await,
        Some(decimal("2"))
    );
    assert!(fixture.queue.records().is_empty());
    assert!(logs.contains("Unable to enqueue budget window spend update"));
}
