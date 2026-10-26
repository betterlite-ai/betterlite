use super::{Charge, UsageRow};
use crate::focus_records::table::ColumnValues;

pub(super) const COLUMNS: [(&str, Source); 44] = [
    ("BilledCost", Source::Cost),
    ("BillingAccountId", Source::Field(Field::ApiKey)),
    ("BillingAccountName", Source::Field(Field::ApiKeyAlias)),
    ("BillingAccountType", Source::Label(Label::ApiKey)),
    ("BillingCurrency", Source::Label(Label::Usd)),
    ("BillingPeriodEnd", Source::PeriodEnd),
    ("BillingPeriodStart", Source::PeriodStart),
    ("ChargeCategory", Source::Label(Label::Usage)),
    ("ChargeClass", Source::NullText),
    ("ChargeDescription", Source::Field(Field::Model)),
    ("ChargeFrequency", Source::Label(Label::UsageBased)),
    ("ChargePeriodEnd", Source::PeriodEnd),
    ("ChargePeriodStart", Source::PeriodStart),
    ("ConsumedQuantity", Source::Quantity),
    ("ConsumedUnit", Source::Label(Label::Requests)),
    ("ContractedCost", Source::Cost),
    // LiteLLM fills this one with a null string, not the schema's Decimal
    ("ContractedUnitPrice", Source::NullText),
    ("EffectiveCost", Source::Cost),
    ("InvoiceIssuerName", Source::Field(Field::Provider)),
    ("InvoiceId", Source::NullText),
    ("ListCost", Source::Cost),
    ("ListUnitPrice", Source::NullDecimal),
    ("AvailabilityZone", Source::NullText),
    ("PricingCurrency", Source::Label(Label::Usd)),
    ("PricingCategory", Source::NullText),
    ("PricingQuantity", Source::Quantity),
    ("PricingCurrencyContractedUnitPrice", Source::NullDecimal),
    ("PricingCurrencyEffectiveCost", Source::Cost),
    ("PricingCurrencyListUnitPrice", Source::NullDecimal),
    ("PricingUnit", Source::Label(Label::Requests)),
    ("ProviderName", Source::Field(Field::Provider)),
    ("PublisherName", Source::Field(Field::Provider)),
    ("RegionId", Source::NullText),
    ("RegionName", Source::NullText),
    ("ResourceId", Source::Field(Field::Model)),
    ("ResourceName", Source::Field(Field::Model)),
    ("ResourceType", Source::Field(Field::Model)),
    ("ServiceCategory", Source::Label(Label::MachineLearning)),
    ("ServiceSubcategory", Source::Label(Label::GenerativeAi)),
    ("ServiceName", Source::Field(Field::ModelGroup)),
    ("SubAccountId", Source::Field(Field::TeamId)),
    ("SubAccountName", Source::Field(Field::TeamAlias)),
    ("SubAccountType", Source::NullText),
    ("Tags", Source::Tags),
];

#[derive(Clone, Copy)]
pub(super) enum Field {
    ApiKey,
    ApiKeyAlias,
    Model,
    ModelGroup,
    Provider,
    TeamId,
    TeamAlias,
}

impl Field {
    fn value(self, row: &UsageRow) -> Option<String> {
        match self {
            Self::ApiKey => Some(row.api_key.clone()),
            Self::ApiKeyAlias => row.api_key_alias.clone(),
            Self::Model => row.model.clone(),
            Self::ModelGroup => row.model_group.clone(),
            Self::Provider => row.custom_llm_provider.clone(),
            Self::TeamId => row.team_id.clone(),
            Self::TeamAlias => row.team_alias.clone(),
        }
    }
}

#[derive(Clone, Copy)]
pub(super) enum Label {
    ApiKey,
    Usd,
    Usage,
    UsageBased,
    Requests,
    MachineLearning,
    GenerativeAi,
}

impl Label {
    const fn text(self) -> &'static str {
        match self {
            Self::ApiKey => "API Key",
            Self::Usd => "USD",
            Self::Usage => "Usage",
            Self::UsageBased => "Usage-Based",
            Self::Requests => "Requests",
            Self::MachineLearning => "AI and Machine Learning",
            Self::GenerativeAi => "Generative AI",
        }
    }
}

#[derive(Clone, Copy)]
pub(super) enum Source {
    Field(Field),
    Label(Label),
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
        match self {
            Self::Field(field) => ColumnValues::Text(
                charges
                    .iter()
                    .map(|charge| field.value(charge.row))
                    .collect(),
            ),
            Self::Label(label) => {
                ColumnValues::Text(vec![Some(label.text().to_owned()); charges.len()])
            }
            Self::NullText => ColumnValues::Text(vec![None; charges.len()]),
            Self::NullDecimal => ColumnValues::Decimal(vec![None; charges.len()]),
            Self::Cost => {
                ColumnValues::Decimal(charges.iter().map(|charge| Some(charge.cost)).collect())
            }
            Self::Quantity => {
                ColumnValues::Decimal(charges.iter().map(|charge| Some(charge.quantity)).collect())
            }
            Self::PeriodStart => ColumnValues::Text(
                charges
                    .iter()
                    .map(|charge| charge.period_start.clone())
                    .collect(),
            ),
            Self::PeriodEnd => ColumnValues::Text(
                charges
                    .iter()
                    .map(|charge| charge.period_end.clone())
                    .collect(),
            ),
            Self::Tags => ColumnValues::Text(
                charges
                    .iter()
                    .map(|charge| Some(charge.tags.clone()))
                    .collect(),
            ),
        }
    }
}
