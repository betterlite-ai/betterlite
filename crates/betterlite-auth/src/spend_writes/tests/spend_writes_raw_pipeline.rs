use crate::spend_counter_batch::batch::PendingSpendIncrement;
use crate::spend_reseed::values::ReseedCacheValue;
use crate::spend_writes::error::{LayersError, SpendWriteError};
use crate::spend_writes::pipeline::run_spend_counter_pipeline;

use super::support::{Fault, KEY, OTHER_KEY, decimal, fixture, increment};

fn pending_pair() -> [PendingSpendIncrement; 2] {
    [increment(KEY, "2"), increment(OTHER_KEY, "3")]
}

#[tokio::test]
async fn the_raw_pipeline_returns_the_redis_values_in_order() {
    let fx = fixture(true);
    fx.layers.answer_pipeline(vec![decimal("5"), decimal("7")]);
    let counted = run_spend_counter_pipeline(&fx.writes, &pending_pair())
        .await
        .expect("the raw pipeline answers");
    assert_eq!(counted, vec![Some(decimal("5")), Some(decimal("7"))]);
    assert_eq!(
        fx.layers.memory_value(KEY),
        Some(ReseedCacheValue::Number(decimal("5")))
    );
    assert_eq!(
        fx.layers.memory_value(OTHER_KEY),
        Some(ReseedCacheValue::Number(decimal("7")))
    );
    assert_eq!(fx.journal.count("batch_forget:"), 0);
}

#[tokio::test]
async fn the_raw_pipeline_writes_only_the_counters_the_reply_covers() {
    let fx = fixture(true);
    fx.layers.answer_pipeline(vec![decimal("5")]);
    let counted = run_spend_counter_pipeline(&fx.writes, &pending_pair())
        .await
        .expect("a short reply answers");
    assert_eq!(counted, vec![Some(decimal("5"))]);
    assert_eq!(
        fx.layers.memory_value(KEY),
        Some(ReseedCacheValue::Number(decimal("5")))
    );
    assert_eq!(fx.layers.memory_value(OTHER_KEY), None);
    assert_eq!(fx.batch.recorded(), vec![(KEY.to_owned(), decimal("5"))]);
}

#[tokio::test]
async fn the_raw_pipeline_records_every_counter_after_the_redis_reply() {
    let fx = fixture(true);
    let counted = run_spend_counter_pipeline(&fx.writes, &pending_pair())
        .await
        .expect("the raw pipeline answers");
    assert_eq!(counted, vec![Some(decimal("2")), Some(decimal("3"))]);
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
async fn the_raw_pipeline_without_redis_increments_each_counter_locally() {
    let fx = fixture(false);
    let counted = run_spend_counter_pipeline(&fx.writes, &pending_pair())
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
async fn the_raw_pipeline_leaves_cleanup_to_the_caller() {
    let fx = fixture(true);
    fx.layers
        .put_memory(KEY, ReseedCacheValue::Number(decimal("1")));
    fx.layers.faults.add(Fault::Pipeline);
    let error = run_spend_counter_pipeline(&fx.writes, &pending_pair())
        .await
        .expect_err("the pipeline failure surfaces");
    assert!(matches!(
        error,
        SpendWriteError::Layers(LayersError::Redis(_))
    ));
    assert!(error.is_transient());
    assert_eq!(fx.journal.events(), vec!["pipeline:2".to_owned()]);
    assert_eq!(
        fx.layers.memory_value(KEY),
        Some(ReseedCacheValue::Number(decimal("1")))
    );
}

#[tokio::test]
async fn an_empty_raw_pipeline_answers_empty_without_touching_a_layer() {
    let fx = fixture(true);
    let counted = run_spend_counter_pipeline(&fx.writes, &[])
        .await
        .expect("an empty pipeline answers");
    assert!(counted.is_empty());
    assert!(fx.journal.events().is_empty());
}
