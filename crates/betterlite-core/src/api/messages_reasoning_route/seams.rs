use betterlite_models::decisions::Json;
use betterlite_models::full_model_metadata::record::ModelInfo;
use betterlite_models::model_metadata_resolution::names::ModelCostMap;
use indexmap::IndexMap;
use thiserror::Error;

use crate::responses_bridge::ModelInfoError;

// get_llm_provider's four arguments; None is a value it is given as null, and the seam decides what that means
pub struct ProviderLookup<'lookup> {
    pub model: Option<&'lookup str>,
    pub custom_llm_provider: Option<&'lookup str>,
    pub api_base: Option<&'lookup str>,
    pub api_key: Option<&'lookup str>,
}

// get_llm_provider's four return values, less the dynamic key this path never reads
pub struct ResolvedTarget {
    pub model: String,
    pub provider: String,
    pub api_base: Option<String>,
}

pub trait ProviderResolution {
    fn resolve(&self, lookup: &ProviderLookup<'_>) -> Option<ResolvedTarget>;
}

// get_model_info plus the map resolve_supported_efforts reads a bare entry's per-level flags from
pub trait ModelMetadata {
    fn model_info(
        &self,
        model: Option<&str>,
        custom_llm_provider: Option<&str>,
    ) -> Option<ModelInfo>;

    fn model_cost(&self) -> &ModelCostMap;
}

// responses_api_bridge_check's arguments, with the search options and tools already filtered to the shapes it accepts
pub struct BridgeRequest<'request> {
    pub model: &'request str,
    pub custom_llm_provider: &'request str,
    pub web_search_options: Option<&'request Json>,
    pub tools: Option<&'request [Json]>,
    pub reasoning_effort: &'request IndexMap<String, Json>,
    pub api_base: Option<&'request str>,
}

// the policy's answer; only mode is consumed here, the rewritten model belongs to the policy's owner
pub struct BridgeOutcome {
    pub mode: Option<String>,
    pub model: String,
}

pub trait BridgePolicy {
    fn bridge(&self, request: &BridgeRequest<'_>) -> Result<BridgeOutcome, ModelInfoError>;
}

// the three lookups B174, B205 and B158 own, injected so this module never rebuilds them
pub struct ReasoningSeams<'seams> {
    pub provider: &'seams dyn ProviderResolution,
    pub metadata: &'seams dyn ModelMetadata,
    pub bridge: &'seams dyn BridgePolicy,
}

#[derive(Debug, Error)]
pub enum ReasoningRouteError {
    #[error("the responses bridge policy failed")]
    Bridge {
        #[from]
        source: ModelInfoError,
    },
}
