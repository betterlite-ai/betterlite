use rust_decimal::Decimal;
use serde::Deserialize;

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct SearchModelInfo {
    pub input_cost_per_query: Option<Decimal>,
    pub tiered_pricing: Option<Vec<PricingTier>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct PricingTier {
    pub max_results_range: (i64, i64),
    pub input_cost_per_query: Decimal,
}
