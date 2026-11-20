use std::convert::Infallible;

use betterlite_models::full_model_metadata::parameters::SupportedParameterSource;
use betterlite_models::projections::reasoning_effort::ThinkingBudgetThresholds;

use crate::api::api_boundaries::ApiLimits;

// What the caller resolved for one adapter call; the provider is only the one the caller named.
#[derive(Clone, Copy)]
pub struct AdapterSettings<'settings> {
    pub custom_llm_provider: Option<&'settings str>,
    pub auto_summary: bool,
    pub preserve_midturn_system: bool,
    pub thresholds: ThinkingBudgetThresholds,
    pub limits: ApiLimits,
    pub parameters: &'settings dyn SupportedParameterSource<Error = Infallible>,
}

impl<'settings> AdapterSettings<'settings> {
    // The defaults LiteLLM's keyword arguments carry, so a caller names only the settings it resolved.
    #[must_use]
    pub fn new(parameters: &'settings dyn SupportedParameterSource<Error = Infallible>) -> Self {
        Self {
            custom_llm_provider: None,
            auto_summary: false,
            preserve_midturn_system: false,
            thresholds: ThinkingBudgetThresholds::default(),
            limits: ApiLimits::default(),
            parameters,
        }
    }
}
