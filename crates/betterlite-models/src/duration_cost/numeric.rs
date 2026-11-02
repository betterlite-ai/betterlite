use rust_decimal::Decimal;

use super::error::DurationCostError;
use crate::price_lookup::PriceRecord;
use crate::tiered_pricing::{Scalar, coerce_cost_per_token};

// A present-but-null field and an absent one both fall through to the caller's next candidate.
pub(super) fn numeric_price(
    record: &PriceRecord,
    key: &str,
) -> Result<Option<Decimal>, DurationCostError> {
    match record.get(key) {
        None | Some(Scalar::Null) => Ok(None),
        present => Ok(Some(coerce_cost_per_token(present)?)),
    }
}

mod tests;
