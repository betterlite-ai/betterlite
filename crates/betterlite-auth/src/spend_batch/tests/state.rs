use std::sync::Arc;

use crate::spend_batch::batch::{SpendCounterBatch, SpendCounterValue};
use crate::spend_batch::increment::PendingSpendIncrement;

use super::support::{FakeRedis, allow_cache, decimal, keys};

async fn open_batch(server: &FakeRedis) -> Arc<SpendCounterBatch> {
    SpendCounterBatch::new(allow_cache(server).await)
}

#[tokio::test]
async fn record_binds_the_key_and_serves_it_without_a_fetch() {
    let server = FakeRedis::start().await;
    let batch = open_batch(&server).await;
    batch.record("spend:key:a", decimal("5"));
    assert_eq!(
        batch.read("spend:key:a").await.expect("the read succeeds"),
        Some(SpendCounterValue::Amount(decimal("5")))
    );
    assert!(server.commands_named("MGET").is_empty());
}

#[tokio::test]
async fn forget_removes_a_recorded_and_a_bound_key() {
    let server = FakeRedis::start().await;
    let batch = open_batch(&server).await;
    batch.bind(&keys(&["spend:key:b"]));
    batch.record("spend:key:a", decimal("5"));
    batch.forget("spend:key:a");
    batch.forget("spend:key:b");
    assert_eq!(
        batch.read("spend:key:a").await.expect("the read succeeds"),
        None
    );
    assert_eq!(
        batch.read("spend:key:b").await.expect("the read succeeds"),
        None
    );
    assert!(server.commands_named("MGET").is_empty());
}

#[test]
fn a_pending_increment_keeps_its_key_and_signed_amount() {
    let increment = PendingSpendIncrement {
        counter_key: "spend:key:a".to_owned(),
        increment: decimal("-5"),
    };
    assert_eq!(increment.counter_key, "spend:key:a");
    assert_eq!(increment.increment, decimal("-5"));
    assert!(increment.increment.is_sign_negative());
}
