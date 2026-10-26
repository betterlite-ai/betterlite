#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ColumnType {
    Decimal,
    Text,
    Timestamp,
}

pub const DECIMAL_PRECISION: u8 = 18;
pub const DECIMAL_SCALE: u8 = 6;

pub const FOCUS_NORMALIZED_SCHEMA: [(&str, ColumnType); 37] = [
    ("BilledCost", ColumnType::Decimal),
    ("BillingAccountId", ColumnType::Text),
    ("BillingAccountName", ColumnType::Text),
    ("BillingCurrency", ColumnType::Text),
    ("BillingPeriodStart", ColumnType::Timestamp),
    ("BillingPeriodEnd", ColumnType::Timestamp),
    ("ChargeCategory", ColumnType::Text),
    ("ChargeClass", ColumnType::Text),
    ("ChargeDescription", ColumnType::Text),
    ("ChargeFrequency", ColumnType::Text),
    ("ChargePeriodStart", ColumnType::Timestamp),
    ("ChargePeriodEnd", ColumnType::Timestamp),
    ("ConsumedQuantity", ColumnType::Decimal),
    ("ConsumedUnit", ColumnType::Text),
    ("ContractedCost", ColumnType::Decimal),
    ("ContractedUnitPrice", ColumnType::Decimal),
    ("EffectiveCost", ColumnType::Decimal),
    ("InvoiceIssuerName", ColumnType::Text),
    ("ListCost", ColumnType::Decimal),
    ("ListUnitPrice", ColumnType::Decimal),
    ("PricingCategory", ColumnType::Text),
    ("PricingQuantity", ColumnType::Decimal),
    ("PricingUnit", ColumnType::Text),
    ("ProviderName", ColumnType::Text),
    ("PublisherName", ColumnType::Text),
    ("RegionId", ColumnType::Text),
    ("RegionName", ColumnType::Text),
    ("ResourceId", ColumnType::Text),
    ("ResourceName", ColumnType::Text),
    ("ResourceType", ColumnType::Text),
    ("ServiceCategory", ColumnType::Text),
    ("ServiceSubcategory", ColumnType::Text),
    ("ServiceName", ColumnType::Text),
    ("SubAccountId", ColumnType::Text),
    ("SubAccountName", ColumnType::Text),
    ("SubAccountType", ColumnType::Text),
    // LiteLLM holds the tag metadata Vantage reads as a JSON string here
    ("Tags", ColumnType::Text),
];

mod tests;
