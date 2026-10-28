use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ProviderSpecificPricing {
    #[serde(deserialize_with = "us")]
    pub us: Option<Decimal>,
    #[serde(deserialize_with = "fast")]
    pub fast: Option<Decimal>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MetadataPricing {
    #[serde(rename = "original_pricing_per_hour")]
    #[serde(deserialize_with = "per_hour")]
    pub per_hour: Option<Decimal>,
    #[serde(rename = "original_pricing_per_minute")]
    #[serde(deserialize_with = "per_minute")]
    pub per_minute: Option<Decimal>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SamplePricing {
    #[serde(rename = "computer_use_input_cost_per_1k_tokens")]
    #[serde(deserialize_with = "computer_use_input")]
    pub computer_use_input: Option<Decimal>,
    #[serde(rename = "computer_use_output_cost_per_1k_tokens")]
    #[serde(deserialize_with = "computer_use_output")]
    pub computer_use_output: Option<Decimal>,
    #[serde(rename = "file_search_cost_per_1k_calls")]
    #[serde(deserialize_with = "file_search_calls")]
    pub file_search_calls: Option<Decimal>,
    #[serde(rename = "file_search_cost_per_gb_per_day")]
    #[serde(deserialize_with = "file_search_storage")]
    pub file_search_storage: Option<Decimal>,
    #[serde(rename = "vector_store_cost_per_gb_per_day")]
    #[serde(deserialize_with = "vector_store_storage")]
    pub vector_store_storage: Option<Decimal>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct OffPeakPricing {
    #[serde(deserialize_with = "input_cost_per_token")]
    pub input_cost_per_token: Option<Decimal>,
    #[serde(deserialize_with = "output_cost_per_token")]
    pub output_cost_per_token: Option<Decimal>,
    #[serde(deserialize_with = "cache_read_input_token_cost")]
    pub cache_read_input_token_cost: Option<Decimal>,
    #[serde(
        rename = "cache_creation_input_token_cost",
        skip_serializing_if = "Option::is_none"
    )]
    #[serde(deserialize_with = "cache_create_token")]
    pub cache_create_token: Option<Decimal>,
    #[serde(
        rename = "output_cost_per_reasoning_token",
        skip_serializing_if = "Option::is_none"
    )]
    #[serde(deserialize_with = "output_reasoning_token")]
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
    #[serde(deserialize_with = "search_context_size_low")]
    pub search_context_size_low: Option<Decimal>,
    #[serde(deserialize_with = "search_context_size_medium")]
    pub search_context_size_medium: Option<Decimal>,
    #[serde(deserialize_with = "search_context_size_high")]
    pub search_context_size_high: Option<Decimal>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TierPricing {
    #[serde(rename = "input_cost_per_token")]
    #[serde(deserialize_with = "input_token")]
    pub input_token: Option<Decimal>,
    #[serde(rename = "output_cost_per_token")]
    #[serde(deserialize_with = "output_token")]
    pub output_token: Option<Decimal>,
    #[serde(rename = "output_cost_per_reasoning_token")]
    #[serde(deserialize_with = "output_reasoning_token")]
    pub output_reasoning_token: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(deserialize_with = "cache_read_input_token_cost")]
    pub cache_read_input_token_cost: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(deserialize_with = "input_cost_per_query")]
    pub input_cost_per_query: Option<Decimal>,
    #[serde(
        rename = "cache_creation_input_token_cost",
        skip_serializing_if = "Option::is_none"
    )]
    #[serde(deserialize_with = "cache_create_token")]
    pub cache_create_token: Option<Decimal>,
    #[serde(
        deserialize_with = "serde_with::As::<Option<Vec<serde_with::FromInto<super::deserialize::ExactDecimal>>>>::deserialize"
    )]
    pub range: Option<Vec<Decimal>>,
    #[serde(
        deserialize_with = "serde_with::As::<Option<Vec<serde_with::FromInto<super::deserialize::ExactDecimal>>>>::deserialize"
    )]
    pub max_results_range: Option<Vec<Decimal>>,
}

macro_rules! price_reader {
    ($($name:ident: $wire:literal),* $(,)?) => {
        $(
            fn $name<'de, Deserializer>(
                deserializer: Deserializer,
            ) -> Result<Option<Decimal>, Deserializer::Error>
                where Deserializer: serde::Deserializer<'de>
                {
                let price = super::deserialize::read_price(deserializer, $wire)?;
                Ok(Some(price))
            }
        )*
    };
}

price_reader! {
    us: "us",
    fast: "fast",
    per_hour: "original_pricing_per_hour",
    per_minute: "original_pricing_per_minute",
    computer_use_input: "computer_use_input_cost_per_1k_tokens",
    computer_use_output: "computer_use_output_cost_per_1k_tokens",
    file_search_calls: "file_search_cost_per_1k_calls",
    file_search_storage: "file_search_cost_per_gb_per_day",
    vector_store_storage: "vector_store_cost_per_gb_per_day",
    input_cost_per_token: "input_cost_per_token",
    output_cost_per_token: "output_cost_per_token",
    cache_read_input_token_cost: "cache_read_input_token_cost",
    cache_create_token: "cache_creation_input_token_cost",
    output_reasoning_token: "output_cost_per_reasoning_token",
    search_context_size_low: "search_context_size_low",
    search_context_size_medium: "search_context_size_medium",
    search_context_size_high: "search_context_size_high",
    input_token: "input_cost_per_token",
    output_token: "output_cost_per_token",
    input_cost_per_query: "input_cost_per_query",
}
