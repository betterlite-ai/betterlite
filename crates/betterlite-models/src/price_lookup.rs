use crate::tiered_pricing::Scalar;
use std::collections::BTreeMap;

// Each field keyed by its wire name; a value may be non-numeric.
pub type PriceRecord = BTreeMap<String, Scalar>;

// Shared by cost leaves so each one does not grow its own cache or model-name resolver.
pub trait PriceLookup {
    // Soft lookup: a failed model or provider resolution comes back absent, never an error (B205).
    fn effective_model_info(
        &self,
        model: &str,
        custom_llm_provider: Option<&str>,
    ) -> Option<PriceRecord>;

    // Raw lookup: an exact cost-map entry by its own key, never synthesized from resolved metadata (B193).
    fn raw_cost_map_entry(&self, key: &str) -> Option<PriceRecord>;
}

mod tests;
