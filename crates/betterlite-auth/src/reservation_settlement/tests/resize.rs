use rust_decimal::Decimal;

use crate::reservation_settlement::error::SettlementError;
use crate::reservation_settlement::resize::resize_applied_reservation;

use super::support::{counters, decimal, entry, take_calls, value_of};

#[tokio::test]
async fn a_resize_moves_every_entry_to_the_new_cost() {
    let fake = counters(&[("spend:key:k1", "10"), ("spend:key:k2", "8")]);
    let mut second = entry("spend:key:k2", "10");
    second.applied_adjustment = decimal("-2");
    let mut entries = vec![entry("spend:key:k1", "10"), second];
    resize_applied_reservation(&fake, &mut entries, decimal("10"), decimal("6"))
        .await
        .expect("the resize");
    let expected = vec![vec![
        ("spend:key:k1".to_owned(), decimal("-4")),
        ("spend:key:k2".to_owned(), decimal("-2")),
    ]];
    assert_eq!(take_calls(&fake).pipelines, expected);
    assert_eq!(entries[0].reserved_cost, Some(decimal("6")));
    assert_eq!(entries[1].reserved_cost, Some(decimal("6")));
    assert_eq!(entries[0].applied_adjustment, Decimal::ZERO);
    assert_eq!(entries[1].applied_adjustment, Decimal::ZERO);
    assert_eq!(value_of(&fake, "spend:key:k1"), Some(decimal("6")));
}

#[tokio::test]
async fn a_resize_refuses_an_inconsistent_counter() {
    let fake = counters(&[("spend:key:k1", "10")]);
    let mut entries = vec![entry("spend:key:k1", "10"), entry("spend:key:k2", "10")];
    let error = resize_applied_reservation(&fake, &mut entries, decimal("10"), decimal("6"))
        .await
        .expect_err("the refusal");
    assert!(
        matches!(&error, SettlementError::InconsistentCounter { counter_key } if counter_key == "spend:key:k2")
    );
    let calls = take_calls(&fake);
    assert!(calls.pipelines.is_empty());
    assert!(calls.reseeds.is_empty());
    assert_eq!(entries[0].reserved_cost, Some(decimal("10")));
    assert_eq!(entries[1].reserved_cost, Some(decimal("10")));
}
