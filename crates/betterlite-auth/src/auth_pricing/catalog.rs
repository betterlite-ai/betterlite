use std::fmt::Debug;

use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use thiserror::Error;

use crate::auth_key_budgets::money::Money;

use super::cache::ZeroCostCache;

// the two resolved per-token rates get_model_group_info carries; absent means an undefined cost
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ModelGroupCosts {
    pub input_cost_per_token: Option<Money>,
    pub output_cost_per_token: Option<Money>,
}

// one deployment as get_model_list and get_deployment_model_info expose it
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PricingDeployment {
    pub litellm_params: JsonObject,
    pub model_info: JsonObject,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Error)]
pub enum PricingError {
    #[error("the model catalog could not answer")]
    Unavailable,
}

// the router catalog these checks read, including its zero-cost cache
pub trait PricingCatalog: Debug + Send + Sync {
    fn model_group_info(&self, model: &str) -> Result<Option<ModelGroupCosts>, PricingError>;

    fn model_list(&self, model: &str) -> Result<Vec<PricingDeployment>, PricingError>;

    fn deployment_model_info(
        &self,
        model_id: &str,
        model_name: &str,
    ) -> Result<Option<JsonObject>, PricingError>;

    fn raw_cost_entry(&self, model_id: &str) -> Option<&JsonObject>;

    fn zero_cost_cache(&self) -> Option<&ZeroCostCache>;
}

// a deployment's model_info.id as LiteLLM's dict keys it; anything but a string misses the cost map
#[must_use]
pub fn model_info_id(model_info: &JsonObject) -> Option<&str> {
    match model_info.get("id") {
        Some(Json::Text(id)) => Some(id),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))
        | None => None,
    }
}
