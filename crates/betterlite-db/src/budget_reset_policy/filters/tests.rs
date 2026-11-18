#![cfg(test)]

use super::{LINKED_KEYS, SPENT_ROWS};
use crate::spend_reset_unit_of_work::queue::SpendRange;
use rust_decimal::Decimal;

#[test]
fn filters_spent_rows_select_strictly_positive_spend() {
    assert_eq!(SPENT_ROWS.greater_than, Some(Decimal::ZERO));
    assert_eq!(SPENT_ROWS.less_than_or_equal, None);
    assert_ne!(SPENT_ROWS, SpendRange::default());
}

#[test]
fn filters_linked_keys_use_the_spent_rows_filter() {
    assert_eq!(LINKED_KEYS, SPENT_ROWS);
    assert_eq!(LINKED_KEYS.greater_than, Some(Decimal::ZERO));
}
