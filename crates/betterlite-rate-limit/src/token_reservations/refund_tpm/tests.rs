#![cfg(test)]

use std::collections::BTreeSet;

use crate::token_reservations::tests::support::{
    ReservationFixture, pipeline_tuples, scope, tokens_key,
};

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
        pipeline_tuples(&fixture.effects.pipelines()),
        vec![vec![
            (
                tokens_key("model_per_key", "key1:gpt-4"),
                -100.0_f64,
                Some(60)
            ),
            (
                tokens_key("model_per_team", "team1:gpt-4"),
                -100.0_f64,
                Some(60)
            ),
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
        pipeline_tuples(&fixture.effects.pipelines()),
        vec![vec![
            (
                tokens_key("model_per_key", "key1:gpt-4"),
                -100.0_f64,
                Some(60)
            ),
            (
                tokens_key("model_per_key", "key1:gpt-4"),
                -100.0_f64,
                Some(60)
            ),
        ]]
    );
}
