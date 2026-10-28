use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ProviderSpecificPricing {
    pub us: Option<Decimal>,
    pub fast: Option<Decimal>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MetadataPricing {
    #[serde(rename = "original_pricing_per_hour")]
    pub per_hour: Option<Decimal>,
    #[serde(rename = "original_pricing_per_minute")]
    pub per_minute: Option<Decimal>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SamplePricing {
    #[serde(rename = "computer_use_input_cost_per_1k_tokens")]
    pub computer_use_input: Option<Decimal>,
    #[serde(rename = "computer_use_output_cost_per_1k_tokens")]
    pub computer_use_output: Option<Decimal>,
    #[serde(rename = "file_search_cost_per_1k_calls")]
    pub file_search_calls: Option<Decimal>,
    #[serde(rename = "file_search_cost_per_gb_per_day")]
    pub file_search_storage: Option<Decimal>,
    #[serde(rename = "vector_store_cost_per_gb_per_day")]
    pub vector_store_storage: Option<Decimal>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct OffPeakPricing {
    pub input_cost_per_token: Option<Decimal>,
    pub output_cost_per_token: Option<Decimal>,
    pub cache_read_input_token_cost: Option<Decimal>,
    #[serde(
        rename = "cache_creation_input_token_cost",
        skip_serializing_if = "Option::is_none"
    )]
    pub cache_create_token: Option<Decimal>,
    #[serde(
        rename = "output_cost_per_reasoning_token",
        skip_serializing_if = "Option::is_none"
    )]
    pub output_reasoning_token: Option<Decimal>,
    pub windows: Option<Vec<PriceWindow>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hours_utc: Option<HoursUtc>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PriceWindow {
    pub hours_utc: HoursUtc,
    pub weekdays: Vec<Weekday>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum HoursUtc {
    One(String),
    Many(Vec<String>),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Weekday {
    Name(String),
    Number(u8),
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SearchContextPricing {
    pub search_context_size_low: Option<Decimal>,
    pub search_context_size_medium: Option<Decimal>,
    pub search_context_size_high: Option<Decimal>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TierPricing {
    #[serde(rename = "input_cost_per_token")]
    pub input_token: Option<Decimal>,
    #[serde(rename = "output_cost_per_token")]
    pub output_token: Option<Decimal>,
    #[serde(rename = "output_cost_per_reasoning_token")]
    pub output_reasoning_token: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_read_input_token_cost: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_cost_per_query: Option<Decimal>,
    #[serde(
        rename = "cache_creation_input_token_cost",
        skip_serializing_if = "Option::is_none"
    )]
    pub cache_create_token: Option<Decimal>,
    pub range: Option<Vec<Decimal>>,
    pub max_results_range: Option<Vec<Decimal>>,
}
