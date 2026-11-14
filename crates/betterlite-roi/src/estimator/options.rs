use betterlite_models::model_capability_predicates::{
    CapabilityPredicates, CapabilityRequest, NamedCapability,
};
use serde::{Deserialize, Serialize};

use crate::settings::ROISettings;

use super::models::{EstimatorModel, ReasoningEffort};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EstimatorOptions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<ReasoningEffort>,
}

pub async fn estimator_options<Predicates>(
    predicates: &mut Predicates,
    models: &[EstimatorModel],
) -> EstimatorOptions
where
    Predicates: CapabilityPredicates + Send,
{
    if models.is_empty() {
        return EstimatorOptions::default();
    }
    for (model, provider) in models {
        let request = CapabilityRequest {
            model,
            custom_llm_provider: provider.as_deref(),
        };
        let supported = predicates
            .supports_factory(&request, NamedCapability::NoneReasoningEffort.key())
            .await;
        if !supported {
            return EstimatorOptions::default();
        }
    }
    EstimatorOptions {
        reasoning_effort: Some(ReasoningEffort::None),
    }
}

#[must_use]
pub fn configured_models(
    settings: &ROISettings,
    models: Option<&[EstimatorModel]>,
) -> Vec<EstimatorModel> {
    models.map_or_else(
        || vec![(settings.estimator_model.clone(), None)],
        |resolved| resolved.to_vec(),
    )
}
