use betterlite_models::billed_tokens::BilledTokenUsage;
use betterlite_models::billed_tokens::custom::CustomTokenUsage;
use betterlite_models::off_peak::schema::OffPeakBlock;
use betterlite_models::price_lookup::PriceRecord;
use betterlite_models::tiered_pricing::Tier;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::Deserialize;

#[derive(Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Operation {
    #[default]
    CustomCost,
    CustomBreakdown,
    Counts,
    Catalog,
    Savings,
}

#[derive(Deserialize)]
pub(super) struct CustomRates {
    pub(super) input_cost_per_token: Decimal,
    pub(super) output_cost_per_token: Decimal,
    pub(super) cache_read_input_token_cost: Option<Decimal>,
    #[serde(rename = "cache_creation_input_token_cost")]
    pub(super) cache_write: Option<Decimal>,
}

#[derive(Deserialize)]
pub(crate) struct Input {
    #[serde(default)]
    pub(super) operation: Operation,
    #[serde(flatten)]
    pub(super) totals: CustomTokenUsage,
    pub(super) custom_cost_per_token: Option<CustomRates>,
    pub(super) custom_cost_per_second: Option<Decimal>,
    pub(super) response_time_ms: Option<Decimal>,
    #[serde(default)]
    pub(super) usage: BilledTokenUsage,
    #[serde(default)]
    pub(super) model_info: ModelInfo,
    #[serde(default)]
    pub(super) tiered_pricing: Vec<Tier>,
    pub(super) off_peak_pricing: Option<OffPeakBlock>,
    pub(super) provider: Option<String>,
    pub(super) service_tier: Option<String>,
    pub(super) residency: Option<String>,
    pub(super) vertex_location: Option<String>,
    pub(super) inference_geo: Option<String>,
    #[serde(default = "default_moment")]
    pub(super) moment: DateTime<Utc>,
}

const fn default_moment() -> DateTime<Utc> {
    DateTime::UNIX_EPOCH
}

#[derive(Default, Deserialize)]
pub(super) struct ModelInfo {
    pub(super) provider_specific_entry: Option<PriceRecord>,
    #[serde(flatten)]
    pub(super) prices: PriceRecord,
}
