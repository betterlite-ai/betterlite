use std::sync::Arc;

use betterlite_redis::request_batch::lock;
use rstest::rstest;
use tokio::task::JoinSet;

use crate::spend_counter_batch::batch::{CounterValue, PendingSpendIncrement, SpendCounterBatch};

use super::support::{DebugEvents, FakeRedis, Reply, allow_cache, decimal, keys};

async fn open_batch(server: &FakeRedis) -> Arc<SpendCounterBatch> {
    SpendCounterBatch::new(allow_cache(server).await)
}

#[tokio::test]
async fn float_counter_keeps_every_digit() {
    let server = FakeRedis::start().await;
    let batch = open_batch(&server).await;
    batch.bind(&keys(&["spend:key:a"]));
    server.script_mget(Reply::Array(vec![Reply::Bulk(
        "0.30000000000000004".to_owned(),
    )]));
    assert_eq!(
        batch.read("spend:key:a").await.unwrap(),
        Some(CounterValue::Amount(decimal("0.30000000000000004")))
    );
}

#[tokio::test]
async fn long_numeric_string_keeps_cents() {
    let server = FakeRedis::start().await;
    let batch = open_batch(&server).await;
    batch.bind(&keys(&["spend:key:a"]));
    server.script_mget(Reply::Array(vec![Reply::Bulk(
        "\"12345678901234567.89\"".to_owned(),
    )]));
    assert_eq!(
        batch.read("spend:key:a").await.unwrap(),
        Some(CounterValue::Amount(decimal("12345678901234567.89")))
    );
}

#[test]
fn negative_increment_allowed() {
    let increment = PendingSpendIncrement {
        counter_key: "spend:key:a".to_owned(),
        increment: decimal("-5"),
    };
    assert_eq!(increment.counter_key, "spend:key:a");
    assert_eq!(increment.increment, decimal("-5"));
    assert!(increment.increment.is_sign_negative());
}

#[tokio::test]
async fn exact_decimal_counter_merge() {
    let server = FakeRedis::start().await;
    let batch = open_batch(&server).await;
    batch.bind(&keys(&["spend:key:a"]));
    server.script_mget(Reply::Array(vec![Reply::Bulk(
        "0.30000000000000004".to_owned(),
    )]));
    let mut held = server.hold_next_mget();
    let mut tasks = JoinSet::new();
    let reading = Arc::clone(&batch);
    tasks.spawn(async move { reading.read("spend:key:a").await });
    held.wait_arrived().await;
    let exact = decimal("0.3");
    batch.record("spend:key:a", exact);
    held.release();
    let value = tasks.join_next().await.unwrap().unwrap().unwrap();
    assert_eq!(value, Some(CounterValue::Amount(exact)));
    let stored = {
        let state = lock(&batch.state);
        state.loaded.get("spend:key:a").copied()
    };
    assert_eq!(stored, Some(Some(exact)));
}

#[rstest]
#[case::nan("\"nan\"")]
#[case::infinity("\"inf\"")]
#[case::negative_infinity("\"-inf\"")]
#[tokio::test]
async fn nonfinite_map_debug_fallback(#[case] payload: &str) {
    let server = FakeRedis::start().await;
    let events = DebugEvents::capture();
    let batch = open_batch(&server).await;
    batch.bind(&keys(&["spend:key:a"]));
    server.script_mget(Reply::Array(vec![Reply::Bulk(payload.to_owned())]));
    assert!(batch.read("spend:key:a").await.unwrap().is_none());
    assert!(events.count() >= 1);
}

#[rstest]
#[case::exponent_overflow("\"1e400\"")]
#[case::beyond_decimal_max("1e300")]
#[tokio::test]
async fn out_of_range_map_debug_fallback(#[case] payload: &str) {
    let server = FakeRedis::start().await;
    let events = DebugEvents::capture();
    let batch = open_batch(&server).await;
    batch.bind(&keys(&["spend:key:a"]));
    server.script_mget(Reply::Array(vec![Reply::Bulk(payload.to_owned())]));
    assert!(batch.read("spend:key:a").await.unwrap().is_none());
    assert!(events.count() >= 1);
}
