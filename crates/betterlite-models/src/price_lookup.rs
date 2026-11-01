use crate::off_peak::schema::OffPeakBlock;
use crate::tiered_pricing::{Scalar, Tier};
use std::collections::BTreeMap;

// Each field keyed by its wire name; a value may be non-numeric.
pub type PriceRecord = BTreeMap<String, Scalar>;

// Carries a resolved entry's record with the tiered and off-peak tables PriceRecord cannot hold.
#[derive(Debug, Clone, Default)]
pub struct PricedModel {
    pub record: PriceRecord,
    pub tiered_pricing: Vec<Tier>,
    pub off_peak_pricing: Option<OffPeakBlock>,
    pub off_peak_cache_read_present: bool,
}

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

    // Resolves like effective_model_info but keeps the entry's tiered and off-peak tables decoded.
    fn effective_priced_model(
        &self,
        model: &str,
        custom_llm_provider: Option<&str>,
    ) -> Option<PricedModel>;
}

mod tests;
