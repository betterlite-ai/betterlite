#![cfg(test)]

use super::focus_normalized_schema;
use polars::prelude::{DataType, TimeUnit};

#[test]
fn empty_schema_has_37_columns() {
    let schema = focus_normalized_schema();
    assert_eq!(schema.len(), 37);
    assert_eq!(schema.get("Tags").unwrap(), &DataType::String);
    assert_eq!(
        schema.get("BilledCost").unwrap(),
        &DataType::Decimal(Some(18), Some(6))
    );
    assert_eq!(
        schema.get("ChargePeriodStart").unwrap(),
        &DataType::Datetime(TimeUnit::Microseconds, None)
    );
}
