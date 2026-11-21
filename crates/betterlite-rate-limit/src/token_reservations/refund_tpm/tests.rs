#![cfg(test)]

use std::collections::BTreeSet;

use betterlite_redis::cache_primitives::pipeline_increment::IncrementOperation;

use crate::token_reservations::tests::support::{ReservationFixture, scope, tokens_key};

fn payment(key: &str, value: &str, amount: f64) -> IncrementOperation {
    IncrementOperation {
        key: tokens_key(key, value),
        increment_value: amount,
        ttl: Some(60),
    }
}

#[tokio::test]
async fn refunds_one_pipeline_operation_per_scope() {
    let fixture = ReservationFixture::seeded(&[]);

    fixture
        .reservations()
        .refund_reserved_tokens(
            &[
                scope("model_per_key", "key1:gpt-4"),
                scope("model_per_team", "team1:gpt-4"),
            ],
            100,
            &BTreeSet::new(),
        )
        .await
        .unwrap();

    assert_eq!(
        fixture.effects.pipelines(),
        vec![vec![
            payment("model_per_key", "key1:gpt-4", -100.0_f64),
            payment("model_per_team", "team1:gpt-4", -100.0_f64),
        ]]
    );
    assert!(fixture.effects.reads().is_empty());
    assert!(fixture.effects.writes().is_empty());
    assert!(fixture.effects.increments().is_empty());
}

#[tokio::test]
async fn refunds_a_scope_named_twice_twice() {
    let fixture = ReservationFixture::seeded(&[]);

    fixture
        .reservations()
        .refund_reserved_tokens(
            &[
                scope("model_per_key", "key1:gpt-4"),
                scope("model_per_key", "key1:gpt-4"),
            ],
            100,
            &BTreeSet::new(),
        )
        .await
        .unwrap();

    assert_eq!(
        fixture.effects.pipelines(),
        vec![vec![
            payment("model_per_key", "key1:gpt-4", -100.0_f64),
            payment("model_per_key", "key1:gpt-4", -100.0_f64),
        ]]
    );
}
