use std::time::Duration;

use rust_decimal::Decimal;

use super::support::{
    FixedJitter, RecordingPause, ScriptedWriter, WriteFailure, key, rollup, transaction,
};
use crate::model_usage::{
    DEFAULT_RETRY_TIMES, ModelUsageError, ModelUsageFlushError, ModelUsageRollup,
    ModelUsageTransaction, flush_model_usage_transactions,
};

fn one_transaction() -> ModelUsageTransaction {
    transaction(
        key("2026-01-02", "gpt-4o", "gpt-4o", "openai", "uncategorized"),
        Decimal::new(25, 2),
        (10, 5),
        true,
    )
}

fn one_rollup() -> ModelUsageRollup {
    rollup(one_transaction(), (1, 1, 0))
}

#[tokio::test]
async fn one_flush_writes_one_additive_batch() {
    let writer = ScriptedWriter::new(Vec::<WriteFailure>::new());
    let pause = RecordingPause::default();
    let jitter = FixedJitter { nanos: 0 };
    let transactions = [
        one_transaction(),
        transaction(
            key("2026-01-02", "gpt-4o", "gpt-4o", "openai", "uncategorized"),
            Decimal::new(25, 2),
            (20, 10),
            false,
        ),
    ];
    flush_model_usage_transactions(&writer, &transactions, DEFAULT_RETRY_TIMES, &jitter, &pause)
        .await
        .expect("the flush writes");
    assert_eq!(
        writer.batches(),
        [vec![rollup(
            transaction(
                key("2026-01-02", "gpt-4o", "gpt-4o", "openai", "uncategorized"),
                Decimal::new(5, 1),
                (30, 15),
                false,
            ),
            (2, 1, 1)
        )]]
    );
    assert!(pause.delays().is_empty());
}

#[tokio::test]
async fn an_empty_flush_writes_nothing() {
    let writer = ScriptedWriter::new(Vec::<WriteFailure>::new());
    let pause = RecordingPause::default();
    let jitter = FixedJitter { nanos: 0 };
    flush_model_usage_transactions(&writer, &[], DEFAULT_RETRY_TIMES, &jitter, &pause)
        .await
        .expect("nothing to write");
    assert!(writer.batches().is_empty());
    assert!(pause.delays().is_empty());
}

#[tokio::test]
async fn a_pre_send_failure_is_retried_with_doubling_delays() {
    let writer = ScriptedWriter::new([
        WriteFailure::PreSend,
        WriteFailure::PreSend,
        WriteFailure::PreSend,
    ]);
    let pause = RecordingPause::default();
    let jitter = FixedJitter { nanos: 0 };
    flush_model_usage_transactions(
        &writer,
        &[one_transaction()],
        DEFAULT_RETRY_TIMES,
        &jitter,
        &pause,
    )
    .await
    .expect("the fourth attempt writes");
    assert_eq!(writer.batches(), vec![vec![one_rollup()]; 4]);
    assert_eq!(
        pause.delays(),
        [
            Duration::from_secs(1),
            Duration::from_secs(2),
            Duration::from_secs(4),
        ]
    );
}

#[tokio::test]
async fn a_pre_send_failure_returns_once_the_retries_run_out() {
    let writer = ScriptedWriter::new([WriteFailure::PreSend; 4]);
    let pause = RecordingPause::default();
    let jitter = FixedJitter { nanos: 0 };
    let outcome = flush_model_usage_transactions(
        &writer,
        &[one_transaction()],
        DEFAULT_RETRY_TIMES,
        &jitter,
        &pause,
    )
    .await;
    assert!(matches!(
        outcome,
        Err(ModelUsageFlushError::Write(WriteFailure::PreSend))
    ));
    assert_eq!(writer.batches().len(), 4);
    assert_eq!(pause.delays().len(), 3);
}

#[tokio::test]
async fn an_ambiguous_failure_returns_without_a_retry() {
    let writer = ScriptedWriter::new([WriteFailure::AfterSend]);
    let pause = RecordingPause::default();
    let jitter = FixedJitter { nanos: 0 };
    let outcome =
        flush_model_usage_transactions(&writer, &[one_transaction()], 5, &jitter, &pause).await;
    assert!(matches!(
        outcome,
        Err(ModelUsageFlushError::Write(WriteFailure::AfterSend))
    ));
    assert_eq!(writer.batches().len(), 1);
    assert!(pause.delays().is_empty());
}

#[tokio::test]
async fn a_failure_after_a_retry_may_have_landed_and_stops_the_flush() {
    let writer = ScriptedWriter::new([WriteFailure::PreSend, WriteFailure::AfterSend]);
    let pause = RecordingPause::default();
    let jitter = FixedJitter { nanos: 0 };
    let outcome =
        flush_model_usage_transactions(&writer, &[one_transaction()], 5, &jitter, &pause).await;
    assert!(matches!(
        outcome,
        Err(ModelUsageFlushError::Write(WriteFailure::AfterSend))
    ));
    assert_eq!(writer.batches().len(), 2);
    assert_eq!(pause.delays().len(), 1);
}

#[tokio::test]
async fn the_jitter_stretches_each_delay_by_less_than_one_second() {
    let writer = ScriptedWriter::new([WriteFailure::PreSend; 3]);
    let pause = RecordingPause::default();
    let jitter = FixedJitter { nanos: 999_999_999 };
    let outcome =
        flush_model_usage_transactions(&writer, &[one_transaction()], 2, &jitter, &pause).await;
    assert!(matches!(
        outcome,
        Err(ModelUsageFlushError::Write(WriteFailure::PreSend))
    ));
    assert_eq!(writer.batches().len(), 3);
    assert_eq!(
        pause.delays(),
        [
            Duration::from_nanos(1_999_999_999),
            Duration::from_nanos(2_999_999_999),
        ]
    );
}

#[tokio::test]
async fn a_rollup_that_cannot_aggregate_never_reaches_the_writer() {
    let largest = transaction(
        key("2026-01-02", "gpt-4o", "gpt-4o", "openai", "uncategorized"),
        Decimal::MAX,
        (0, 0),
        true,
    );
    let writer = ScriptedWriter::new(Vec::<WriteFailure>::new());
    let pause = RecordingPause::default();
    let jitter = FixedJitter { nanos: 0 };
    let outcome = flush_model_usage_transactions(
        &writer,
        &[largest.clone(), largest],
        DEFAULT_RETRY_TIMES,
        &jitter,
        &pause,
    )
    .await;
    assert!(matches!(
        outcome,
        Err(ModelUsageFlushError::Aggregate(
            ModelUsageError::SpendOverflow
        ))
    ));
    assert!(writer.batches().is_empty());
    assert!(pause.delays().is_empty());
}

#[test]
fn the_default_retry_count_matches_the_source() {
    assert_eq!(DEFAULT_RETRY_TIMES, 3);
}
