#![cfg(test)]

use std::collections::BTreeSet;

use rstest::rstest;

use crate::token_reservations::tests::support::{ReservationFixture, scope};

#[rstest]
#[case::zero_amount(0, true)]
#[case::negative_amount(-50, true)]
#[case::no_scopes(100, false)]
#[case::zero_amount_and_no_scopes(0, false)]
#[tokio::test]
async fn refunds_nothing_when_the_amount_or_the_scopes_are_missing(
    #[case] amount: i64,
    #[case] with_scope: bool,
) {
    let fixture = ReservationFixture::seeded(&[]);
    let scopes = if with_scope {
        vec![scope("model_per_key", "key1:gpt-4")]
    } else {
        Vec::new()
    };

    fixture
        .reservations()
        .refund_reserved_tokens(&scopes, amount, &BTreeSet::new())
        .await
        .unwrap();

    assert!(fixture.effects.reads().is_empty());
    assert!(fixture.effects.writes().is_empty());
    assert!(fixture.effects.increments().is_empty());
    assert!(fixture.effects.pipelines().is_empty());
}

#[tokio::test]
async fn refunds_a_positive_amount_for_a_named_scope() {
    let fixture = ReservationFixture::seeded(&[]);

    fixture
        .reservations()
        .refund_reserved_tokens(&[scope("model_per_key", "key1:gpt-4")], 1, &BTreeSet::new())
        .await
        .unwrap();

    assert_eq!(fixture.effects.pipelines().len(), 1);
}
