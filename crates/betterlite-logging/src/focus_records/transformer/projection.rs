use super::{decimal, formatted_datetime};
use crate::focus_records::schema::DECIMAL;
use polars::prelude::{Column, DataFrame, DataType, PlSmallStr, PolarsError};

pub(super) const COLUMNS: [(&str, Source); 44] = [
    ("BilledCost", Source::Money),
    ("BillingAccountId", Source::Field("api_key")),
    ("BillingAccountName", Source::Field("api_key_alias")),
    ("BillingAccountType", Source::Text("API Key")),
    ("BillingCurrency", Source::Text("USD")),
    ("BillingPeriodEnd", Source::Period("ChargePeriodEnd")),
    ("BillingPeriodStart", Source::Period("ChargePeriodStart")),
    ("ChargeCategory", Source::Text("Usage")),
    ("ChargeClass", Source::Null(DataType::String)),
    ("ChargeDescription", Source::Field("model")),
    ("ChargeFrequency", Source::Text("Usage-Based")),
    ("ChargePeriodEnd", Source::Period("ChargePeriodEnd")),
    ("ChargePeriodStart", Source::Period("ChargePeriodStart")),
    ("ConsumedQuantity", Source::Quantity),
    ("ConsumedUnit", Source::Text("Requests")),
    ("ContractedCost", Source::Money),
    ("ContractedUnitPrice", Source::Null(DataType::String)),
    ("EffectiveCost", Source::Money),
    ("InvoiceIssuerName", Source::Field("custom_llm_provider")),
    ("InvoiceId", Source::Null(DataType::String)),
    ("ListCost", Source::Money),
    ("ListUnitPrice", Source::Null(DECIMAL)),
    ("AvailabilityZone", Source::Null(DataType::String)),
    ("PricingCurrency", Source::Text("USD")),
    ("PricingCategory", Source::Null(DataType::String)),
    ("PricingQuantity", Source::Quantity),
    ("PricingCurrencyContractedUnitPrice", Source::Null(DECIMAL)),
    ("PricingCurrencyEffectiveCost", Source::Money),
    ("PricingCurrencyListUnitPrice", Source::Null(DECIMAL)),
    ("PricingUnit", Source::Text("Requests")),
    ("ProviderName", Source::Field("custom_llm_provider")),
    ("PublisherName", Source::Field("custom_llm_provider")),
    ("RegionId", Source::Null(DataType::String)),
    ("RegionName", Source::Null(DataType::String)),
    ("ResourceId", Source::Field("model")),
    ("ResourceName", Source::Field("model")),
    ("ResourceType", Source::Field("model")),
    ("ServiceCategory", Source::Text("AI and Machine Learning")),
    ("ServiceSubcategory", Source::Text("Generative AI")),
    ("ServiceName", Source::Field("model_group")),
    ("SubAccountId", Source::Field("team_id")),
    ("SubAccountName", Source::Field("team_alias")),
    ("SubAccountType", Source::Null(DataType::String)),
    ("Tags", Source::Tags),
];

pub(super) enum Source {
    Field(&'static str),
    Text(&'static str),
    Null(DataType),
    Money,
    Quantity,
    Period(&'static str),
    Tags,
}

impl Source {
    pub(super) fn project(&self, frame: &DataFrame) -> Result<Column, PolarsError> {
        match self {
            Self::Field(field) => frame.column(field)?.cast(&DataType::String),
            Self::Text(text) => Ok(Column::new(PlSmallStr::EMPTY, vec![*text; frame.height()])),
            Self::Null(dtype) => Ok(Column::full_null(PlSmallStr::EMPTY, frame.height(), dtype)),
            Self::Money => decimal(frame, "spend"),
            Self::Quantity => decimal(frame, "api_requests"),
            Self::Period(source) => formatted_datetime(frame, source),
            Self::Tags => frame.column("Tags")?.cast(&DataType::String),
        }
    }
}
