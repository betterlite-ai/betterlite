use polars::prelude::{DataType, PlSmallStr, Schema, TimeUnit};

pub const DECIMAL: DataType = DataType::Decimal(Some(18), Some(6));
pub const FOCUS_COLUMN_COUNT: usize = 37;

const COLUMNS: [(&str, DataType); FOCUS_COLUMN_COUNT] = [
    ("BilledCost", DECIMAL),
    ("BillingAccountId", DataType::String),
    ("BillingAccountName", DataType::String),
    ("BillingCurrency", DataType::String),
    (
        "BillingPeriodStart",
        DataType::Datetime(TimeUnit::Microseconds, None),
    ),
    (
        "BillingPeriodEnd",
        DataType::Datetime(TimeUnit::Microseconds, None),
    ),
    ("ChargeCategory", DataType::String),
    ("ChargeClass", DataType::String),
    ("ChargeDescription", DataType::String),
    ("ChargeFrequency", DataType::String),
    (
        "ChargePeriodStart",
        DataType::Datetime(TimeUnit::Microseconds, None),
    ),
    (
        "ChargePeriodEnd",
        DataType::Datetime(TimeUnit::Microseconds, None),
    ),
    ("ConsumedQuantity", DECIMAL),
    ("ConsumedUnit", DataType::String),
    ("ContractedCost", DECIMAL),
    ("ContractedUnitPrice", DECIMAL),
    ("EffectiveCost", DECIMAL),
    ("InvoiceIssuerName", DataType::String),
    ("ListCost", DECIMAL),
    ("ListUnitPrice", DECIMAL),
    ("PricingCategory", DataType::String),
    ("PricingQuantity", DECIMAL),
    ("PricingUnit", DataType::String),
    ("ProviderName", DataType::String),
    ("PublisherName", DataType::String),
    ("RegionId", DataType::String),
    ("RegionName", DataType::String),
    ("ResourceId", DataType::String),
    ("ResourceName", DataType::String),
    ("ResourceType", DataType::String),
    ("ServiceCategory", DataType::String),
    ("ServiceSubcategory", DataType::String),
    ("ServiceName", DataType::String),
    ("SubAccountId", DataType::String),
    ("SubAccountName", DataType::String),
    ("SubAccountType", DataType::String),
    ("Tags", DataType::String),
];

#[must_use]
pub fn focus_normalized_schema() -> Schema {
    let fields = COLUMNS
        .into_iter()
        .map(|(name, dtype)| (PlSmallStr::from(name), dtype));
    Schema::from_iter(fields)
}

mod tests;
