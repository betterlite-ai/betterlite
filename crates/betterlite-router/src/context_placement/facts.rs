use betterlite_models::catalog::ModelCatalog;
use betterlite_models::decisions::Json;
use betterlite_models::identity::declared::authenticating_provider;
use betterlite_models::lookup::info::ModelQuery;
use betterlite_models::lookup::resolve::{find_info, split_model};
use betterlite_tokenizer::model_capabilities::BEDROCK_NAMES;

use crate::complexity_math::window_holds;
use crate::deployment_identity::spec::DeploymentSpec;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModelLimitKey {
    InputTokens,
    OutputTokens,
}

impl ModelLimitKey {
    #[must_use]
    pub const fn field(self) -> &'static str {
        match self {
            Self::InputTokens => "max_input_tokens",
            Self::OutputTokens => "max_output_tokens",
        }
    }
}

// (smallest declared window across a group, whether any deployment declares none)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WindowFacts {
    pub window: Option<u64>,
    pub has_unknown: bool,
}

impl WindowFacts {
    pub const UNKNOWN: Self = Self {
        window: None,
        has_unknown: true,
    };

    // _group_provably_fits: escalation needs a declared window on every deployment of the group
    #[must_use]
    pub fn provably_fits(self, needed: u64, buffer: f64) -> bool {
        match self {
            Self {
                window: Some(window),
                has_unknown: false,
            } => window_holds(Some(window), Some(needed), buffer),
            Self { .. } => false,
        }
    }
}

// _deployment_limit's get_router_model_info: the limit the router's model map declares for a deployment
pub trait RouterModelInfo: Send + Sync {
    fn limit(&self, group: &str, deployment: &DeploymentSpec, key: ModelLimitKey) -> Option<u64>;
}

#[derive(Clone, Copy)]
pub struct CatalogModelInfo<'catalog> {
    pub catalog: &'catalog ModelCatalog,
}

impl RouterModelInfo for CatalogModelInfo<'_> {
    fn limit(&self, _group: &str, deployment: &DeploymentSpec, key: ModelLimitKey) -> Option<u64> {
        let model = text(deployment.litellm_params.get("model"))?;
        let base_model = text(deployment.model_info.get("base_model"))
            .or_else(|| text(deployment.litellm_params.get("base_model")));
        let provider_override = text(deployment.litellm_params.get("custom_llm_provider"))
            .filter(|provider| !provider.is_empty());
        let split = split_model(self.catalog, model);
        let provider = provider_override
            .map(str::to_owned)
            .or_else(|| split.provider.clone())?;
        // get_router_model_info resolves azure by base_model and ignores the deployment's own model
        let requested = if provider.as_str() == "azure" {
            base_model.unwrap_or(split.model.as_str())
        } else {
            split.model.as_str()
        };
        let prefix = format!("{provider}/");
        let model_info_name = if requested.starts_with(prefix.as_str()) {
            requested.to_owned()
        } else {
            format!("{provider}/{requested}")
        };
        let query = ModelQuery {
            model: &model_info_name,
            provider: Some(provider.as_str()),
        };
        let resolved = find_info(self.catalog, &BEDROCK_NAMES, &query)
            .ok()
            .flatten()?;
        match key {
            ModelLimitKey::InputTokens => resolved.limits.max_input_tokens,
            ModelLimitKey::OutputTokens => resolved.limits.max_output_tokens,
        }
    }
}

// _deployment_limit: a declared limit shadows the model map even when it is no integer, an authenticating provider has none, and only an absent or null declaration asks the map
#[must_use]
pub fn deployment_limit(
    model_info: &dyn RouterModelInfo,
    group: &str,
    deployment: &DeploymentSpec,
    key: ModelLimitKey,
) -> Option<u64> {
    match deployment.model_info.get(key.field()) {
        None | Some(Json::Null) => {}
        Some(Json::Number(number)) => return number.as_u64(),
        Some(_) => return None,
    }
    let model = text(deployment.litellm_params.get("model"));
    let provider_override = text(deployment.litellm_params.get("custom_llm_provider"))
        .filter(|provider| !provider.is_empty());
    if authenticating_provider(model, provider_override).is_some() {
        return None;
    }
    model_info.limit(group, deployment, key)
}

fn text(value: Option<&Json>) -> Option<&str> {
    match value {
        Some(Json::Text(text)) => Some(text),
        _ => None,
    }
}
