use std::sync::Arc;

use crate::spend_counter_batch::batch::PendingSpendIncrement;
use crate::spend_reseed::values::ReseedCacheValue;
use crate::spend_writes::error::{LayersError, SpendWriteError};
use crate::spend_writes::layers::CounterLayers;

use super::refusing::refusing_redis_cache;
use super::support::{Failure, Fault, KEY, OTHER_KEY, decimal, dual_cache, fixture, increment};

fn pending_pair() -> [PendingSpendIncrement; 2] {
    [increment(KEY, "2"), increment(OTHER_KEY, "3")]
}

#[tokio::test]
async fn the_pipeline_answers_each_counter_in_order() {
    let fx = fixture(true);
    let counted = fx
        .writes
        .increment_counters_pipeline(&pending_pair())
        .await
        .expect("the pipeline answers");
    assert_eq!(counted, vec![Some(decimal("2")), Some(decimal("3"))]);
    assert_eq!(
        fx.layers.memory_value(KEY),
        Some(ReseedCacheValue::Number(decimal("2")))
    );
    assert_eq!(
        fx.layers.memory_value(OTHER_KEY),
        Some(ReseedCacheValue::Number(decimal("3")))
    );
    assert_eq!(
        fx.batch.recorded(),
        vec![
            (KEY.to_owned(), decimal("2")),
            (OTHER_KEY.to_owned(), decimal("3")),
        ]
    );
    let expected = vec![
        "pipeline:2".to_owned(),
        format!("memory_set:{KEY}"),
        format!("batch_record:{KEY}:2"),
        format!("memory_set:{OTHER_KEY}"),
        format!("batch_record:{OTHER_KEY}:3"),
    ];
    assert_eq!(fx.journal.events(), expected);
}

#[tokio::test]
async fn the_pipeline_invalidates_every_counter_before_raising() {
    let fx = fixture(true);
    fx.layers
        .put_memory(KEY, ReseedCacheValue::Number(decimal("1")));
    fx.layers
        .put_memory(OTHER_KEY, ReseedCacheValue::Number(decimal("1")));
    fx.layers.faults.add(Fault::Pipeline);
    let error = fx
        .writes
        .increment_counters_pipeline(&pending_pair())
        .await
        .expect_err("the pipeline failure surfaces");
    assert!(matches!(
        error,
        SpendWriteError::Layers(LayersError::Redis(_))
    ));
    assert!(error.is_transient());
    assert_eq!(fx.layers.memory_value(KEY), None);
    assert_eq!(fx.layers.memory_value(OTHER_KEY), None);
    let expected = vec![
        "pipeline:2".to_owned(),
        format!("batch_forget:{KEY}"),
        format!("delete_memory:{KEY}"),
        format!("delete_redis:{KEY}"),
        format!("batch_forget:{OTHER_KEY}"),
        format!("delete_memory:{OTHER_KEY}"),
        format!("delete_redis:{OTHER_KEY}"),
    ];
    assert_eq!(fx.journal.events(), expected);
}

#[tokio::test]
async fn the_pipeline_without_redis_increments_each_counter_locally() {
    let fx = fixture(false);
    let counted = fx
        .writes
        .increment_counters_pipeline(&pending_pair())
        .await
        .expect("the memory pipeline answers");
    assert_eq!(counted, vec![Some(decimal("2")), Some(decimal("3"))]);
    assert_eq!(fx.layers.increment_total(KEY), Some(decimal("2")));
    assert_eq!(fx.layers.increment_total(OTHER_KEY), Some(decimal("3")));
    assert!(fx.batch.recorded().is_empty());
    let expected = vec![
        format!("increment_local:{KEY}"),
        format!("increment_local:{OTHER_KEY}"),
    ];
    assert_eq!(fx.journal.events(), expected);
}

#[tokio::test]
async fn an_empty_pipeline_answers_empty_without_touching_a_layer() {
    let fx = fixture(true);
    let counted = fx
        .writes
        .increment_counters_pipeline(&[])
        .await
        .expect("an empty pipeline answers");
    assert!(counted.is_empty());
    assert!(fx.journal.events().is_empty());
}

#[tokio::test]
async fn a_non_transient_pipeline_failure_still_invalidates() {
    let fx = fixture(true);
    fx.layers.faults.add(Fault::Pipeline);
    *fx.layers.failure.lock().expect("the fake failure lock") = Failure::Unavailable;
    let error = fx
        .writes
        .increment_counters_pipeline(&pending_pair())
        .await
        .expect_err("the pipeline failure surfaces");
    assert!(matches!(
        error,
        SpendWriteError::Layers(LayersError::Unavailable)
    ));
    assert!(!error.is_transient());
    assert_eq!(fx.journal.count("batch_forget:"), 2);
}

#[tokio::test]
async fn the_real_redis_pipeline_requires_a_tier() {
    let cache = dual_cache(None);
    let pending = [increment(KEY, "1")];
    let error = cache
        .increment_redis_pipeline(&pending)
        .await
        .expect_err("the missing tier answers");
    assert!(matches!(error, LayersError::Unavailable));
}

#[tokio::test]
async fn the_real_redis_pipeline_reports_a_refused_increment() {
    let cache = Arc::new(dual_cache(Some(refusing_redis_cache().await)));
    let pending = [increment(KEY, "1")];
    let error = cache
        .increment_redis_pipeline(&pending)
        .await
        .expect_err("the breaker refuses the pipeline");
    assert!(matches!(error, LayersError::Redis(_)));
}
