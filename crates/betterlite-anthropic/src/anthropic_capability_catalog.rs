pub mod capability;
pub mod forced_tool;
pub mod sampling;
mod tests;
pub mod thinking;

use betterlite_config::sections::anthropic_capability_catalog::{
    HIGH_THINKING_BUDGET, MEDIUM_THINKING_BUDGET, XHIGH_THINKING_BUDGET,
};
use betterlite_context::anthropic_catalog_lookup::RawCatalog;
use betterlite_context::anthropic_catalog_lookup::candidates::CandidatePatterns;
use betterlite_models::decisions::Json;
use serde::{Deserialize, Serialize};

// LiteLLM reads litellm.drop_params (global) or the per-call flag; either one enables the drop.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct DropParams {
    pub global: bool,
    pub call: bool,
}

// The xhigh/high/medium budgets betterlite-config resolves from the DEFAULT_REASONING_EFFORT_* variables.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct EffortThresholds {
    pub medium: i64,
    pub high: i64,
    pub xhigh: i64,
}

impl Default for EffortThresholds {
    fn default() -> Self {
        Self {
            medium: MEDIUM_THINKING_BUDGET,
            high: HIGH_THINKING_BUDGET,
            xhigh: XHIGH_THINKING_BUDGET,
        }
    }
}

// The optional_params keys these thinking and effort gates read and rewrite; the rest is AnthropicConfig's.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct OptionalParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<Json>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking: Option<Json>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_config: Option<Json>,
}

// B2216's raw lookup needs the candidate normalizer and both cost maps the caller injected.
pub struct CatalogSources<'source> {
    pub patterns: &'source CandidatePatterns,
    pub runtime: &'source RawCatalog,
    pub packaged: &'source RawCatalog,
}
