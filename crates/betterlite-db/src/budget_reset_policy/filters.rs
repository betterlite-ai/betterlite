use crate::spend_reset_unit_of_work::queue::SpendRange;
use rust_decimal::Decimal;

// _SPENT_ROWS_WHERE: only rows that spent something need a reset.
pub const SPENT_ROWS: SpendRange = SpendRange {
    greater_than: Some(Decimal::ZERO),
    less_than_or_equal: None,
};

// _LINKED_KEYS_WHERE: the same spend filter; the key-linked writer adds budget_duration IS NULL on its own.
pub const LINKED_KEYS: SpendRange = SPENT_ROWS;

mod tests;
