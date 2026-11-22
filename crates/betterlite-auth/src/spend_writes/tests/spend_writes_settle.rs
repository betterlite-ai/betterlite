use std::sync::Arc;

use betterlite_redis::batch::error::BatchError;

use crate::spend_reseed::values::ReseedCacheValue;

use super::support::{
    Failure, FakePostCall, Fault, Fixture, KEY, Logs, decimal, fixture, increment,
};

fn serve_post_call(fx: &Fixture) -> Arc<FakePostCall> {
    let post = Arc::new(FakePostCall::new(60, fx.journal.clone()));
    fx.layers.serve_post_call(Arc::clone(&post));
    post
}

#[tokio::test]
async fn a_settled_value_lands_in_memory_and_the_batch() {
    let logs = Logs::capture();
    let fx = fixture(true);
    let post = serve_post_call(&fx);
    assert!(
        fx.writes
            .defer_spend_counter_increments(&[increment(KEY, "3")])
    );
    post.fire(0, Ok(6.5)).await;
    assert_eq!(
        fx.layers.memory_value(KEY),
        Some(ReseedCacheValue::Number(decimal("6.5")))
    );
    assert_eq!(fx.batch.recorded(), vec![(KEY.to_owned(), decimal("6.5"))]);
    assert!(logs.warnings().is_empty());
}

#[tokio::test]
async fn a_cancelled_settlement_retries_the_in_memory_increment() {
    let logs = Logs::capture();
    let fx = fixture(true);
    fx.layers
        .put_memory(KEY, ReseedCacheValue::Number(decimal("2")));
    let post = serve_post_call(&fx);
    assert!(
        fx.writes
            .defer_spend_counter_increments(&[increment(KEY, "3")])
    );
    post.fire(0, Err(Arc::new(BatchError::Cancelled))).await;
    assert_eq!(fx.layers.increment_total(KEY), Some(decimal("3")));
    assert!(fx.batch.recorded().is_empty());
    assert!(logs.warnings().is_empty());
}

#[tokio::test]
async fn a_cancelled_settlement_leaves_a_missing_counter_missing() {
    let fx = fixture(true);
    let post = serve_post_call(&fx);
    assert!(
        fx.writes
            .defer_spend_counter_increments(&[increment(KEY, "3")])
    );
    post.fire(0, Err(Arc::new(BatchError::Cancelled))).await;
    assert_eq!(fx.layers.increment_total(KEY), None);
    assert_eq!(fx.layers.memory_value(KEY), None);
}

#[tokio::test]
async fn a_failed_settlement_warns_and_invalidates() {
    let logs = Logs::capture();
    let fx = fixture(true);
    fx.layers
        .put_memory(KEY, ReseedCacheValue::Number(decimal("9")));
    fx.layers.put_redis(ReseedCacheValue::Number(decimal("9")));
    let post = serve_post_call(&fx);
    assert!(
        fx.writes
            .defer_spend_counter_increments(&[increment(KEY, "3")])
    );
    let cause = BatchError::MissingReply {
        operation: "INCRBYFLOAT",
    };
    post.fire(0, Err(Arc::new(cause))).await;
    let warnings = logs.warnings();
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].contains("did not land in the post-call pipeline"));
    assert_eq!(fx.layers.memory_value(KEY), None);
    assert_eq!(fx.layers.redis_value(), None);
    assert!(fx.journal.records(&format!("batch_forget:{KEY}")));
}

#[tokio::test]
async fn a_settled_value_without_a_decimal_form_leaves_the_counter_alone() {
    let logs = Logs::capture();
    let fx = fixture(true);
    let post = serve_post_call(&fx);
    assert!(
        fx.writes
            .defer_spend_counter_increments(&[increment(KEY, "3")])
    );
    post.fire(0, Ok(f64::NAN)).await;
    assert_eq!(fx.layers.memory_value(KEY), None);
    assert!(fx.batch.recorded().is_empty());
    assert!(logs.warnings().is_empty());
    assert!(logs.contains("Settled spend counter has no decimal form"));
}

#[tokio::test]
async fn a_settlement_memory_failure_still_records_the_batch_value() {
    let logs = Logs::capture();
    let fx = fixture(true);
    fx.layers.faults.add(Fault::SetMemoryValue);
    *fx.layers.failure.lock().expect("the fake failure lock") = Failure::Memory;
    let post = serve_post_call(&fx);
    assert!(
        fx.writes
            .defer_spend_counter_increments(&[increment(KEY, "3")])
    );
    post.fire(0, Ok(4.0)).await;
    assert_eq!(fx.batch.recorded(), vec![(KEY.to_owned(), decimal("4"))]);
    assert!(logs.contains("Unable to store the settled spend counter in memory"));
    assert!(logs.warnings().is_empty());
}

#[tokio::test]
async fn a_cancelled_settlement_tolerates_an_unreadable_memory_tier() {
    let logs = Logs::capture();
    let fx = fixture(true);
    fx.layers.faults.add(Fault::MemoryGet);
    *fx.layers.failure.lock().expect("the fake failure lock") = Failure::Memory;
    let post = serve_post_call(&fx);
    assert!(
        fx.writes
            .defer_spend_counter_increments(&[increment(KEY, "3")])
    );
    post.fire(0, Err(Arc::new(BatchError::Cancelled))).await;
    assert_eq!(fx.layers.increment_total(KEY), None);
    assert!(logs.contains("after a cancelled increment"));
    assert!(logs.warnings().is_empty());
}

#[tokio::test]
async fn a_cancelled_settlement_tolerates_a_failed_in_memory_increment() {
    let logs = Logs::capture();
    let fx = fixture(true);
    fx.layers
        .put_memory(KEY, ReseedCacheValue::Number(decimal("2")));
    fx.layers.faults.add(Fault::IncrementLocal);
    *fx.layers.failure.lock().expect("the fake failure lock") = Failure::Memory;
    let post = serve_post_call(&fx);
    assert!(
        fx.writes
            .defer_spend_counter_increments(&[increment(KEY, "3")])
    );
    post.fire(0, Err(Arc::new(BatchError::Cancelled))).await;
    assert!(logs.contains("Unable to apply the cancelled spend counter increment"));
    assert!(logs.warnings().is_empty());
}
