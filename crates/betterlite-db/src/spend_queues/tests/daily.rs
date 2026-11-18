use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use rstest::rstest;
use rust_decimal::Decimal;
use tokio::sync::Notify;
use tokio::time::timeout;

use super::support::{
    RecordingSink, capture_logs, daily_row, decimal, update, with_field, without_field,
};
use crate::daily_spend_sql::values::SpendColumnValue;
use crate::spend_queues::SpendQueueError;
use crate::spend_queues::daily::{
    DailySpendUpdateQueue, aggregate_daily_spend_update_transactions,
};
use crate::spend_queues::events::{NoSpendQueueEventSink, SpendQueueService};

#[test]
fn the_first_payload_metadata_survives_and_later_metadata_is_ignored() {
    let second = update(
        "k",
        with_field(
            with_field(
                with_field(
                    daily_row("2.5"),
                    "date",
                    SpendColumnValue::Text("2026-02-02".to_owned()),
                ),
                "model_group",
                SpendColumnValue::Text("other".to_owned()),
            ),
            "failed_requests",
            SpendColumnValue::Int(1),
        ),
    );
    let aggregated =
        aggregate_daily_spend_update_transactions(&[update("k", daily_row("10.0")), second])
            .unwrap();
    let row = aggregated.get("k").unwrap();
    assert_eq!(
        row.get("date"),
        Some(&SpendColumnValue::Text("2026-01-01".to_owned()))
    );
    assert_eq!(
        row.get("api_key"),
        Some(&SpendColumnValue::Text("key".to_owned()))
    );
    assert_eq!(
        row.get("model_group"),
        Some(&SpendColumnValue::Text("group".to_owned()))
    );
    assert_eq!(row.get("prompt_tokens"), Some(&SpendColumnValue::Int(6)));
    assert_eq!(
        row.get("completion_tokens"),
        Some(&SpendColumnValue::Int(4))
    );
    assert_eq!(row.get("api_requests"), Some(&SpendColumnValue::Int(2)));
    assert_eq!(
        row.get("successful_requests"),
        Some(&SpendColumnValue::Int(1))
    );
    assert_eq!(row.get("failed_requests"), Some(&SpendColumnValue::Int(1)));
    assert_eq!(
        row.get("spend"),
        Some(&SpendColumnValue::Decimal(decimal("12.5")))
    );
    assert_eq!(
        row.get("cache_read_input_tokens"),
        Some(&SpendColumnValue::Int(0))
    );
    assert_eq!(
        row.get("autorouter_savings_spend"),
        Some(&SpendColumnValue::Int(0))
    );
}

#[test]
fn a_singleton_keeps_absent_optional_metrics_absent() {
    let row = daily_row("10.0");
    let aggregated =
        aggregate_daily_spend_update_transactions(&[update("k", row.clone())]).unwrap();
    assert_eq!(aggregated.get("k"), Some(&row));
    assert!(!row.contains_key("cache_read_input_tokens"));
    assert!(!row.contains_key("autorouter_savings_spend"));
}

#[test]
fn different_transaction_keys_stay_apart() {
    let aggregated = aggregate_daily_spend_update_transactions(&[
        update("a", daily_row("1.5")),
        update("b", daily_row("2.5")),
    ])
    .unwrap();
    assert_eq!(aggregated.len(), 2);
    assert!(!aggregated.contains_key(""));
}

#[rstest]
#[case::null("cache_read_input_tokens", SpendColumnValue::Null)]
#[case::false_bool("cache_creation_input_tokens", SpendColumnValue::Bool(false))]
#[case::zero("compression_saved_tokens", SpendColumnValue::Int(0))]
#[case::empty_text("total_response_time_ms", SpendColumnValue::Text(String::new()))]
#[case::empty_array("timed_requests", SpendColumnValue::Array(Vec::new()))]
fn a_falsey_optional_counter_adds_zero(
    #[case] field: &'static str,
    #[case] value: SpendColumnValue,
) {
    let second = update("k", with_field(daily_row("1.5"), field, value));
    let aggregated =
        aggregate_daily_spend_update_transactions(&[update("k", daily_row("1.5")), second])
            .unwrap();
    assert_eq!(
        aggregated.get("k").unwrap().get(field),
        Some(&SpendColumnValue::Int(0))
    );
}

#[rstest]
#[case::null("compression_savings_spend", SpendColumnValue::Null)]
#[case::false_bool("prompt_caching_savings_spend", SpendColumnValue::Bool(false))]
#[case::zero("gateway_injected_caching_savings_spend", SpendColumnValue::Int(0))]
#[case::empty_object("autorouter_savings_spend", SpendColumnValue::Object(BTreeMap::new()))]
fn a_falsey_optional_savings_adds_zero(
    #[case] field: &'static str,
    #[case] value: SpendColumnValue,
) {
    let second = update("k", with_field(daily_row("1.5"), field, value));
    let aggregated =
        aggregate_daily_spend_update_transactions(&[update("k", daily_row("1.5")), second])
            .unwrap();
    assert_eq!(
        aggregated.get("k").unwrap().get(field),
        Some(&SpendColumnValue::Int(0))
    );
}

#[test]
fn integer_money_stays_an_integer() {
    let first = update(
        "k",
        with_field(daily_row("1.0"), "spend", SpendColumnValue::Int(10)),
    );
    let second = update(
        "k",
        with_field(daily_row("1.0"), "spend", SpendColumnValue::Int(5)),
    );
    let aggregated = aggregate_daily_spend_update_transactions(&[first, second]).unwrap();
    assert_eq!(
        aggregated.get("k").unwrap().get("spend"),
        Some(&SpendColumnValue::Int(15))
    );
}

#[test]
fn decimal_money_stays_decimal_when_the_other_side_is_an_integer() {
    let first = update(
        "k",
        with_field(
            daily_row("1.5"),
            "spend",
            SpendColumnValue::Decimal(decimal("1.0")),
        ),
    );
    let second = update(
        "k",
        with_field(daily_row("1.5"), "spend", SpendColumnValue::Int(2)),
    );
    let aggregated = aggregate_daily_spend_update_transactions(&[first, second]).unwrap();
    assert_eq!(
        aggregated.get("k").unwrap().get("spend"),
        Some(&SpendColumnValue::Decimal(decimal("3.0")))
    );
}

#[test]
fn a_nonzero_optional_metric_adds_its_own_value() {
    let second = update(
        "k",
        with_field(
            with_field(
                with_field(
                    with_field(
                        with_field(
                            daily_row("1.0"),
                            "cache_read_input_tokens",
                            SpendColumnValue::Int(4),
                        ),
                        "timed_requests",
                        SpendColumnValue::Bool(true),
                    ),
                    "compression_saved_tokens",
                    SpendColumnValue::Decimal(decimal("2.5")),
                ),
                "compression_savings_spend",
                SpendColumnValue::Decimal(decimal("0.5")),
            ),
            "prompt_caching_savings_spend",
            SpendColumnValue::Bool(true),
        ),
    );
    let aggregated =
        aggregate_daily_spend_update_transactions(&[update("k", daily_row("1.0")), second])
            .unwrap();
    let row = aggregated.get("k").unwrap();
    assert_eq!(
        row.get("cache_read_input_tokens"),
        Some(&SpendColumnValue::Int(4))
    );
    assert_eq!(row.get("timed_requests"), Some(&SpendColumnValue::Int(1)));
    assert_eq!(
        row.get("compression_saved_tokens"),
        Some(&SpendColumnValue::Int(2))
    );
    assert_eq!(
        row.get("compression_savings_spend"),
        Some(&SpendColumnValue::Decimal(decimal("0.5")))
    );
    assert_eq!(
        row.get("prompt_caching_savings_spend"),
        Some(&SpendColumnValue::Int(1))
    );
}

#[test]
fn money_is_summed_as_an_exact_decimal() {
    let aggregated = aggregate_daily_spend_update_transactions(&[
        update("k", daily_row("0.1")),
        update("k", daily_row("0.2")),
    ])
    .unwrap();
    assert_eq!(
        aggregated.get("k").unwrap().get("spend"),
        Some(&SpendColumnValue::Decimal(decimal("0.3")))
    );
}

#[test]
fn a_fractional_counter_is_truncated() {
    let first = update(
        "k",
        with_field(
            daily_row("1.0"),
            "prompt_tokens",
            SpendColumnValue::Decimal(decimal("4.9")),
        ),
    );
    let aggregated =
        aggregate_daily_spend_update_transactions(&[first, update("k", daily_row("1.0"))]).unwrap();
    assert_eq!(
        aggregated.get("k").unwrap().get("prompt_tokens"),
        Some(&SpendColumnValue::Int(7))
    );
}

#[test]
fn a_missing_required_incoming_field_is_reported() {
    let second = update("k", without_field(daily_row("1.0"), "spend"));
    assert!(matches!(
        aggregate_daily_spend_update_transactions(&[update("k", daily_row("1.0")), second]),
        Err(SpendQueueError::MissingField { field: "spend" })
    ));
}

#[test]
fn a_missing_required_incoming_counter_is_reported() {
    let second = update("k", without_field(daily_row("1.0"), "prompt_tokens"));
    assert!(matches!(
        aggregate_daily_spend_update_transactions(&[update("k", daily_row("1.0")), second]),
        Err(SpendQueueError::MissingField {
            field: "prompt_tokens"
        })
    ));
}

#[test]
fn a_missing_required_existing_field_is_reported() {
    let first = update("k", without_field(daily_row("1.0"), "prompt_tokens"));
    assert!(matches!(
        aggregate_daily_spend_update_transactions(&[first, update("k", daily_row("1.0"))]),
        Err(SpendQueueError::MissingField {
            field: "prompt_tokens"
        })
    ));
}

#[test]
fn a_missing_required_existing_money_field_is_reported() {
    let first = update("k", without_field(daily_row("1.0"), "spend"));
    assert!(matches!(
        aggregate_daily_spend_update_transactions(&[first, update("k", daily_row("1.0"))]),
        Err(SpendQueueError::MissingField { field: "spend" })
    ));
}

#[test]
fn a_null_required_incoming_value_is_reported() {
    let second = update(
        "k",
        with_field(daily_row("1.0"), "spend", SpendColumnValue::Null),
    );
    assert!(matches!(
        aggregate_daily_spend_update_transactions(&[update("k", daily_row("1.0")), second]),
        Err(SpendQueueError::InvalidAddition { field: "spend" })
    ));
}

#[test]
fn a_null_required_incoming_counter_is_reported() {
    let second = update(
        "k",
        with_field(daily_row("1.0"), "prompt_tokens", SpendColumnValue::Null),
    );
    assert!(matches!(
        aggregate_daily_spend_update_transactions(&[update("k", daily_row("1.0")), second]),
        Err(SpendQueueError::InvalidAddition {
            field: "prompt_tokens"
        })
    ));
}

#[test]
fn a_null_optional_metric_adds_zero() {
    let second = update(
        "k",
        with_field(
            with_field(
                daily_row("1.0"),
                "cache_read_input_tokens",
                SpendColumnValue::Null,
            ),
            "compression_savings_spend",
            SpendColumnValue::Null,
        ),
    );
    let aggregated =
        aggregate_daily_spend_update_transactions(&[update("k", daily_row("1.0")), second])
            .unwrap();
    let row = aggregated.get("k").unwrap();
    assert_eq!(
        row.get("cache_read_input_tokens"),
        Some(&SpendColumnValue::Int(0))
    );
    assert_eq!(
        row.get("compression_savings_spend"),
        Some(&SpendColumnValue::Int(0))
    );
}

#[test]
fn an_existing_null_optional_value_is_reported() {
    let first = update(
        "k",
        with_field(
            daily_row("1.0"),
            "cache_read_input_tokens",
            SpendColumnValue::Null,
        ),
    );
    let second = update(
        "k",
        with_field(
            daily_row("1.0"),
            "cache_read_input_tokens",
            SpendColumnValue::Int(1),
        ),
    );
    assert!(matches!(
        aggregate_daily_spend_update_transactions(&[first, second]),
        Err(SpendQueueError::InvalidAddition {
            field: "cache_read_input_tokens"
        })
    ));
}

#[test]
fn a_text_metric_value_is_reported() {
    let second = update(
        "k",
        with_field(
            daily_row("1.0"),
            "total_response_time_ms",
            SpendColumnValue::Text("slow".to_owned()),
        ),
    );
    assert!(matches!(
        aggregate_daily_spend_update_transactions(&[update("k", daily_row("1.0")), second]),
        Err(SpendQueueError::InvalidAddition {
            field: "total_response_time_ms"
        })
    ));
}

#[test]
fn a_counter_that_overflows_is_reported() {
    let first = update(
        "k",
        with_field(
            daily_row("1.0"),
            "prompt_tokens",
            SpendColumnValue::Int(i64::MAX),
        ),
    );
    assert!(matches!(
        aggregate_daily_spend_update_transactions(&[first, update("k", daily_row("1.0"))]),
        Err(SpendQueueError::FieldOverflow {
            field: "prompt_tokens"
        })
    ));
}

#[test]
fn money_that_overflows_the_decimal_range_is_reported() {
    let first = update(
        "k",
        with_field(
            daily_row("1.0"),
            "spend",
            SpendColumnValue::Decimal(Decimal::MAX),
        ),
    );
    assert!(matches!(
        aggregate_daily_spend_update_transactions(&[first, update("k", daily_row("1.0"))]),
        Err(SpendQueueError::FieldOverflow { field: "spend" })
    ));
}

#[tokio::test]
async fn a_flush_aggregates_rows_and_logs_the_item_count() {
    let logs = capture_logs();
    let queue = DailySpendUpdateQueue::new(8, 100, 10, Arc::new(NoSpendQueueEventSink)).unwrap();
    queue
        .add_update(update("k", daily_row("10.0")))
        .await
        .unwrap();
    queue
        .add_update(update("k", daily_row("2.5")))
        .await
        .unwrap();
    let aggregated = queue
        .flush_and_get_aggregated_daily_spend_update_transactions()
        .await
        .unwrap();
    assert_eq!(
        aggregated.get("k").unwrap().get("spend"),
        Some(&SpendColumnValue::Decimal(decimal("12.5")))
    );
    let text = logs.text();
    assert!(text.contains("flushed daily spend update items from in-memory queue"));
    assert!(text.contains("items=2"));
    queue.shutdown().await;
}

#[tokio::test]
async fn an_empty_flush_returns_nothing_and_logs_nothing() {
    let logs = capture_logs();
    let queue = DailySpendUpdateQueue::new(8, 100, 10, Arc::new(NoSpendQueueEventSink)).unwrap();
    let aggregated = queue
        .flush_and_get_aggregated_daily_spend_update_transactions()
        .await
        .unwrap();
    assert!(aggregated.is_empty());
    assert!(!logs.contains("flushed daily spend update items"));
    queue.shutdown().await;
}

#[tokio::test]
async fn add_update_keeps_rows_out_of_the_debug_log() {
    let logs = capture_logs();
    let queue = DailySpendUpdateQueue::new(8, 100, 10, Arc::new(NoSpendQueueEventSink)).unwrap();
    let secret = with_field(
        daily_row("1.0"),
        "api_key",
        SpendColumnValue::Text("sk-secret".to_owned()),
    );
    queue.add_update(update("k", secret)).await.unwrap();
    let text = logs.text();
    assert!(text.contains("Adding update to the in-memory daily spend queue"));
    assert!(!text.contains("sk-secret"));
    queue.shutdown().await;
}

#[tokio::test]
async fn a_singleton_flush_keeps_absent_metrics_absent() {
    let queue = DailySpendUpdateQueue::new(8, 100, 10, Arc::new(NoSpendQueueEventSink)).unwrap();
    queue
        .add_update(update("k", daily_row("1.0")))
        .await
        .unwrap();
    let aggregated = queue
        .flush_and_get_aggregated_daily_spend_update_transactions()
        .await
        .unwrap();
    assert!(!aggregated.get("k").unwrap().contains_key("timed_requests"));
    queue.shutdown().await;
}

#[tokio::test]
async fn the_daily_queue_compacts_at_the_threshold_and_reports_the_compacted_size() {
    let sink = Arc::new(RecordingSink::default());
    let queue = DailySpendUpdateQueue::new(8, 2, 10, Arc::clone(&sink)).unwrap();
    queue
        .add_update(update("k", daily_row("0.5")))
        .await
        .unwrap();
    queue
        .add_update(update("k", daily_row("0.25")))
        .await
        .unwrap();
    queue
        .add_update(update("k", daily_row("0.25")))
        .await
        .unwrap();
    let aggregated = queue
        .flush_and_get_aggregated_daily_spend_update_transactions()
        .await
        .unwrap();
    assert_eq!(
        aggregated.get("k").unwrap().get("spend"),
        Some(&SpendColumnValue::Decimal(decimal("1.0")))
    );
    queue.shutdown().await;

    let events = sink.events();
    assert!(
        events
            .iter()
            .all(|event| event.service == SpendQueueService::InMemoryDailySpendUpdateQueue)
    );
    let mut sizes = events
        .iter()
        .map(|event| event.queue_size)
        .collect::<Vec<_>>();
    sizes.sort_unstable();
    assert_eq!(sizes, vec![1, 2, 2]);
}

#[tokio::test]
async fn a_flush_waits_for_a_tracked_settlement_and_includes_its_row() {
    let queue =
        Arc::new(DailySpendUpdateQueue::new(8, 100, 10, Arc::new(NoSpendQueueEventSink)).unwrap());
    queue
        .add_update(update("a", daily_row("1.0")))
        .await
        .unwrap();
    let start = Arc::new(Notify::new());
    let settling = Arc::clone(&queue);
    let started = Arc::clone(&start);
    queue.track_interrupted_commit(async move {
        started.notified().await;
        settling
            .add_update(update("b", daily_row("2.0")))
            .await
            .unwrap();
    });
    start.notify_one();
    let aggregated = timeout(
        Duration::from_secs(1),
        queue.flush_and_get_aggregated_daily_spend_update_transactions(),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(aggregated.contains_key("a"));
    assert!(aggregated.contains_key("b"));
    queue.shutdown().await;
}

#[tokio::test]
async fn a_settlement_blocked_on_a_full_queue_is_relieved_by_the_flush() {
    let queue =
        Arc::new(DailySpendUpdateQueue::new(1, 100, 10, Arc::new(NoSpendQueueEventSink)).unwrap());
    queue
        .add_update(update("a", daily_row("1.0")))
        .await
        .unwrap();
    let start = Arc::new(Notify::new());
    let settling = Arc::clone(&queue);
    let started = Arc::clone(&start);
    queue.track_interrupted_commit(async move {
        started.notified().await;
        settling
            .add_update(update("b", daily_row("2.0")))
            .await
            .unwrap();
    });
    start.notify_one();
    let aggregated = timeout(
        Duration::from_secs(1),
        queue.flush_and_get_aggregated_daily_spend_update_transactions(),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(aggregated.len(), 2);
    queue.shutdown().await;
}

#[tokio::test]
async fn a_flush_waits_for_a_settlement_registered_while_it_waits() {
    let queue =
        Arc::new(DailySpendUpdateQueue::new(8, 100, 10, Arc::new(NoSpendQueueEventSink)).unwrap());
    let start = Arc::new(Notify::new());
    let outer = Arc::clone(&queue);
    let inner = Arc::clone(&queue);
    let started = Arc::clone(&start);
    queue.track_interrupted_commit(async move {
        started.notified().await;
        let nested = Arc::clone(&inner);
        outer.track_interrupted_commit(async move {
            nested
                .add_update(update("b", daily_row("2.0")))
                .await
                .unwrap();
        });
        outer
            .add_update(update("a", daily_row("1.0")))
            .await
            .unwrap();
    });
    start.notify_one();
    let aggregated = timeout(
        Duration::from_secs(1),
        queue.flush_and_get_aggregated_daily_spend_update_transactions(),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(aggregated.len(), 2);
    queue.shutdown().await;
}

#[tokio::test]
async fn settling_returns_immediately_when_no_commit_is_tracked() {
    let queue = DailySpendUpdateQueue::new(4, 10, 10, Arc::new(NoSpendQueueEventSink)).unwrap();
    timeout(
        Duration::from_millis(100),
        queue.settle_interrupted_commits(),
    )
    .await
    .unwrap();
    queue.shutdown().await;
}

#[tokio::test]
async fn shutdown_waits_for_settlements_and_delivers_their_events() {
    let sink = Arc::new(RecordingSink::default());
    let queue = Arc::new(DailySpendUpdateQueue::new(8, 100, 10, Arc::clone(&sink)).unwrap());
    queue
        .add_update(update("a", daily_row("1.0")))
        .await
        .unwrap();
    let settled = Arc::new(AtomicBool::new(false));
    let settling = Arc::clone(&queue);
    let done = Arc::clone(&settled);
    queue.track_interrupted_commit(async move {
        settling
            .add_update(update("b", daily_row("2.0")))
            .await
            .unwrap();
        done.store(true, Ordering::SeqCst);
    });
    queue.shutdown().await;
    assert!(settled.load(Ordering::SeqCst));
    let events = sink.events();
    assert_eq!(events.len(), 2);
    assert!(
        events
            .iter()
            .all(|event| event.service == SpendQueueService::InMemoryDailySpendUpdateQueue)
    );
}

#[test]
fn a_capacity_of_at_least_one_is_required() {
    assert!(matches!(
        DailySpendUpdateQueue::new(0, 0, 1, Arc::new(NoSpendQueueEventSink)),
        Err(SpendQueueError::ZeroCapacity)
    ));
}
