pub mod advisory_budget;
pub mod arm;
pub mod pipeline;

mod tests;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rust_decimal::Decimal;

pub struct ShadowSample<'sample> {
    pub request_id: &'sample str,
    pub messages: &'sample [Json],
    pub real_text: &'sample str,
    pub real_model: &'sample str,
    pub real_cost: Decimal,
    pub real_classifier_cost: Decimal,
    pub real_cache_hit: bool,
    pub control_tier: Option<&'sample str>,
    pub shadow_params: &'sample IndexMap<String, Json>,
    pub parent_metadata: &'sample IndexMap<String, Json>,
}
