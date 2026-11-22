use crate::spend_reseed::values::ReseedCacheValue;
use crate::spend_writes::error::{LayersError, SpendWriteError};

use super::support::{Fault, KEY, decimal, fixture};

const SHADOW_KEY: &str = "shadow_eval:leg:abc";

#[tokio::test]
async fn the_public_entry_increments_a_counter_outside_an_entity_scope() {
    let fx = fixture(true);
    let counted = fx
        .writes
        .increment_spend_counter(SHADOW_KEY, decimal("0.25"))
        .await
        .expect("the shadow counter increments");
    assert_eq!(counted, Some(decimal("0.25")));
    assert_eq!(
        fx.layers.memory_value(SHADOW_KEY),
        Some(ReseedCacheValue::Number(decimal("0.25")))
    );
    assert_eq!(
        fx.batch.recorded(),
        vec![(SHADOW_KEY.to_owned(), decimal("0.25"))]
    );
}

#[tokio::test]
async fn the_public_entry_invalidates_on_the_same_failure() {
    let fx = fixture(true);
    fx.layers
        .put_memory(SHADOW_KEY, ReseedCacheValue::Number(decimal("1")));
    fx.layers.faults.add(Fault::IncrementRedis);
    let error = fx
        .writes
        .increment_spend_counter(SHADOW_KEY, decimal("1"))
        .await
        .expect_err("the refused increment surfaces");
    assert!(matches!(
        error,
        SpendWriteError::Layers(LayersError::Redis(_))
    ));
    assert_eq!(fx.layers.memory_value(SHADOW_KEY), None);
    assert!(fx.journal.records(&format!("batch_forget:{SHADOW_KEY}")));
}

#[tokio::test]
async fn the_public_entry_uses_the_memory_tier_without_redis() {
    let fx = fixture(false);
    let counted = fx
        .writes
        .increment_spend_counter(KEY, decimal("3"))
        .await
        .expect("the memory counter increments");
    assert_eq!(counted, Some(decimal("3")));
    assert_eq!(fx.layers.increment_total(KEY), Some(decimal("3")));
}
