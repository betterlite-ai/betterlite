use std::convert::Infallible;

use betterlite_models::full_model_metadata::parameters::SupportedParameterSource;
use betterlite_models::projections::reasoning_effort::ThinkingBudgetThresholds;

use crate::api::api_boundaries::ApiLimits;

// What the caller resolved for this request; the provider is only the one the caller named, never one inferred from the model.
pub struct OptionsContext<'request> {
    pub model: &'request str,
    pub custom_llm_provider: Option<&'request str>,
    pub auto_summary: bool,
    pub thresholds: ThinkingBudgetThresholds,
    pub limits: ApiLimits,
    pub parameters: &'request dyn SupportedParameterSource<Error = Infallible>,
}
