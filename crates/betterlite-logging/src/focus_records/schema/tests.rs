#![cfg(test)]

use super::{ColumnType, DECIMAL_PRECISION, DECIMAL_SCALE, FOCUS_NORMALIZED_SCHEMA};

#[test]
fn empty_schema_has_37_columns_with_litellm_types() {
    let render = |kind: ColumnType| -> String {
        let names = FOCUS_NORMALIZED_SCHEMA
            .iter()
            .filter(|(_, column)| *column == kind);
        names.map(|(name, _)| *name).collect::<Vec<_>>().join(",")
    };
    let decimals = concat!(
        "BilledCost,ConsumedQuantity,ContractedCost,ContractedUnitPrice,EffectiveCost,",
        "ListCost,ListUnitPrice,PricingQuantity"
    );
    let timestamps = "BillingPeriodStart,BillingPeriodEnd,ChargePeriodStart,ChargePeriodEnd";
    assert_eq!(
        (render(ColumnType::Decimal), render(ColumnType::Timestamp)),
        (decimals.to_owned(), timestamps.to_owned())
    );
    assert_eq!(FOCUS_NORMALIZED_SCHEMA.len(), 37);
    assert_eq!(
        FOCUS_NORMALIZED_SCHEMA.last(),
        Some(&("Tags", ColumnType::Text))
    );
    assert_eq!((DECIMAL_PRECISION, DECIMAL_SCALE), (18, 6));
}
