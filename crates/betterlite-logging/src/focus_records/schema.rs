use polars::prelude::{DataType, PlSmallStr, Schema, TimeUnit};

pub fn focus_normalized_schema() -> Schema {
    let names = [
        "BilledCost",
        "BillingAccountId",
        "BillingAccountName",
        "BillingCurrency",
        "BillingPeriodStart",
        "BillingPeriodEnd",
        "ChargeCategory",
        "ChargeClass",
        "ChargeDescription",
        "ChargeFrequency",
        "ChargePeriodStart",
        "ChargePeriodEnd",
        "ConsumedQuantity",
        "ConsumedUnit",
        "ContractedCost",
        "ContractedUnitPrice",
        "EffectiveCost",
        "InvoiceIssuerName",
        "ListCost",
        "ListUnitPrice",
        "PricingCategory",
        "PricingQuantity",
        "PricingUnit",
        "ProviderName",
        "PublisherName",
        "RegionId",
        "RegionName",
        "ResourceId",
        "ResourceName",
        "ResourceType",
        "ServiceCategory",
        "ServiceSubcategory",
        "ServiceName",
        "SubAccountId",
        "SubAccountName",
        "SubAccountType",
        "Tags",
    ];
    let decimal = DataType::Decimal(Some(18), Some(6));
    let datetime = DataType::Datetime(TimeUnit::Microseconds, None);
    let types = [
        decimal.clone(),
        DataType::String,
        DataType::String,
        DataType::String,
        datetime.clone(),
        datetime.clone(),
        DataType::String,
        DataType::String,
        DataType::String,
        DataType::String,
        datetime.clone(),
        datetime.clone(),
        decimal.clone(),
        DataType::String,
        decimal.clone(),
        decimal.clone(),
        decimal.clone(),
        DataType::String,
        decimal.clone(),
        decimal.clone(),
        DataType::String,
        decimal.clone(),
        DataType::String,
        DataType::String,
        DataType::String,
        DataType::String,
        DataType::String,
        DataType::String,
        DataType::String,
        DataType::String,
        DataType::String,
        DataType::String,
        DataType::String,
        DataType::String,
        DataType::String,
        DataType::String,
        DataType::String,
    ];
    Schema::from_iter(
        names
            .into_iter()
            .zip(types)
            .map(|(name, dtype)| (PlSmallStr::from(name), dtype)),
    )
}

pub const FOCUS_COLUMN_COUNT: usize = 37;

#[cfg(test)]
mod tests {
    use super::{focus_normalized_schema, FOCUS_COLUMN_COUNT};
    use polars::prelude::{DataType, SchemaExt};

    #[test]
    fn schema_has_all_focus_columns() {
        let schema = focus_normalized_schema();
        assert_eq!(schema.len(), FOCUS_COLUMN_COUNT);
        assert_eq!(schema.get_field("Tags").map(|field| field.dtype()), Some(&DataType::String));
    }
}
