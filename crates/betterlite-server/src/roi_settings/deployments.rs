use betterlite_roi::estimator::models::EstimatorModel;
use serde::Deserialize;

#[derive(Debug, Clone, Default, Deserialize)]
pub struct RouterEstimatorParams {
    pub model: Option<String>,
    pub base_model: Option<String>,
    pub custom_llm_provider: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct RouterEstimatorModelInfo {
    pub base_model: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RouterEstimatorDeployment {
    pub litellm_params: RouterEstimatorParams,
    pub model_info: Option<RouterEstimatorModelInfo>,
}

pub trait RouterCatalog: Send + Sync {
    fn model_names(&self) -> Vec<String>;
    fn deployments(&self, model_group: &str) -> Vec<RouterEstimatorDeployment>;
}

#[must_use]
pub fn estimator_model(deployment: &RouterEstimatorDeployment) -> Option<EstimatorModel> {
    let parameters = &deployment.litellm_params;
    let model = deployment
        .model_info
        .as_ref()
        .and_then(|info| info.base_model.as_ref())
        .or(parameters.base_model.as_ref())
        .or(parameters.model.as_ref())?;
    Some((model.clone(), parameters.custom_llm_provider.clone()))
}

#[must_use]
pub fn estimator_models(deployments: &[RouterEstimatorDeployment]) -> Vec<EstimatorModel> {
    let mut models = Vec::new();
    for deployment in deployments {
        if let Some(model) = estimator_model(deployment) {
            models.push(model);
        }
    }
    models
}

#[must_use]
pub fn router_estimator_models(
    catalog: Option<&dyn RouterCatalog>,
    model_group: &str,
) -> Vec<EstimatorModel> {
    let Some(router) = catalog else {
        return Vec::new();
    };
    estimator_models(&router.deployments(model_group))
}

#[must_use]
pub fn router_models(catalog: Option<&dyn RouterCatalog>) -> Vec<String> {
    let Some(router) = catalog else {
        return Vec::new();
    };
    let mut names = router.model_names();
    names.sort_unstable();
    names.dedup();
    names
}
