use betterlite_config::types::ModelDeployment;

use crate::routing::benchmark_aggregates::StrategyDeployment;

// the model registry entry the idle-group list reads, owned so the route state can hold it
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfiguredRouter {
    pub model_name: String,
    pub model: Option<String>,
}

#[must_use]
pub fn configured_deployments(model_list: &[ModelDeployment]) -> Vec<ConfiguredRouter> {
    model_list
        .iter()
        .map(|deployment| ConfiguredRouter {
            model_name: deployment.model_name.clone(),
            model: deployment.litellm_params.model.clone(),
        })
        .collect()
}

impl ConfiguredRouter {
    #[must_use]
    pub const fn deployment(&self) -> StrategyDeployment<'_> {
        StrategyDeployment {
            model_name: Some(self.model_name.as_str()),
            model: match &self.model {
                Some(model) => Some(model.as_str()),
                None => None,
            },
        }
    }
}
