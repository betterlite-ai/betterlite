use super::{Charge, UsageRow};
use crate::focus_records::table::ColumnValues;

pub(super) const COLUMNS: [(&str, Source); 44] = [
    ("BilledCost", Source::Cost),
    (
        "BillingAccountId",
        Source::Field(|row| Some(row.api_key.clone())),
    ),
    (
        "BillingAccountName",
        Source::Field(|row| row.api_key_alias.clone()),
    ),
    ("BillingAccountType", Source::Label("API Key")),
    ("BillingCurrency", Source::Label("USD")),
    ("BillingPeriodEnd", Source::PeriodEnd),
    ("BillingPeriodStart", Source::PeriodStart),
    ("ChargeCategory", Source::Label("Usage")),
    ("ChargeClass", Source::NullText),
    ("ChargeDescription", Source::Field(|row| row.model.clone())),
    ("ChargeFrequency", Source::Label("Usage-Based")),
    ("ChargePeriodEnd", Source::PeriodEnd),
    ("ChargePeriodStart", Source::PeriodStart),
    ("ConsumedQuantity", Source::Quantity),
    ("ConsumedUnit", Source::Label("Requests")),
    ("ContractedCost", Source::Cost),
    // LiteLLM fills this one with a null string, not the schema's Decimal
    ("ContractedUnitPrice", Source::NullText),
    ("EffectiveCost", Source::Cost),
    (
        "InvoiceIssuerName",
        Source::Field(|row| row.custom_llm_provider.clone()),
    ),
    ("InvoiceId", Source::NullText),
    ("ListCost", Source::Cost),
    ("ListUnitPrice", Source::NullDecimal),
    ("AvailabilityZone", Source::NullText),
    ("PricingCurrency", Source::Label("USD")),
    ("PricingCategory", Source::NullText),
    ("PricingQuantity", Source::Quantity),
    ("PricingCurrencyContractedUnitPrice", Source::NullDecimal),
    ("PricingCurrencyEffectiveCost", Source::Cost),
    ("PricingCurrencyListUnitPrice", Source::NullDecimal),
    ("PricingUnit", Source::Label("Requests")),
    (
        "ProviderName",
        Source::Field(|row| row.custom_llm_provider.clone()),
    ),
    (
        "PublisherName",
        Source::Field(|row| row.custom_llm_provider.clone()),
    ),
    ("RegionId", Source::NullText),
    ("RegionName", Source::NullText),
    ("ResourceId", Source::Field(|row| row.model.clone())),
    ("ResourceName", Source::Field(|row| row.model.clone())),
    ("ResourceType", Source::Field(|row| row.model.clone())),
    ("ServiceCategory", Source::Label("AI and Machine Learning")),
    ("ServiceSubcategory", Source::Label("Generative AI")),
    ("ServiceName", Source::Field(|row| row.model_group.clone())),
    ("SubAccountId", Source::Field(|row| row.team_id.clone())),
    (
        "SubAccountName",
        Source::Field(|row| row.team_alias.clone()),
    ),
    ("SubAccountType", Source::NullText),
    ("Tags", Source::Tags),
];

#[derive(Clone, Copy)]
pub(super) enum Source {
    Field(fn(&UsageRow) -> Option<String>),
    Label(&'static str),
    NullText,
    NullDecimal,
    Cost,
    Quantity,
    PeriodStart,
    PeriodEnd,
    Tags,
}

impl Source {
    pub(super) fn project(self, charges: &[Charge<'_>]) -> ColumnValues {
        let rows = charges.iter();
        match self {
            Self::Field(field) => {
                ColumnValues::Text(rows.map(|charge| field(charge.row)).collect())
            }
            Self::Label(label) => ColumnValues::Text(vec![Some(label.to_owned()); charges.len()]),
            Self::NullText => ColumnValues::Text(vec![None; charges.len()]),
            Self::NullDecimal => ColumnValues::Decimal(vec![None; charges.len()]),
            Self::Cost => ColumnValues::Decimal(rows.map(|charge| Some(charge.cost)).collect()),
            Self::Quantity => {
                ColumnValues::Decimal(rows.map(|charge| Some(charge.quantity)).collect())
            }
            Self::PeriodStart => {
                ColumnValues::Text(rows.map(|charge| charge.period_start.clone()).collect())
            }
            Self::PeriodEnd => {
                ColumnValues::Text(rows.map(|charge| charge.period_end.clone()).collect())
            }
            Self::Tags => {
                ColumnValues::Text(rows.map(|charge| Some(charge.tags.clone())).collect())
            }
        }
    }
}
