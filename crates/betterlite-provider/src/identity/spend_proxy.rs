use betterlite_models::decisions::Json;
use betterlite_models::identity::declared::authenticating_provider;
use betterlite_models::model_registration::JsonObject;
use betterlite_models::resolution::{ResolutionContext, ResolutionRequest, resolve};

pub const LITELLM_PROXY_PROVIDER: &str = "litellm_proxy";

// should_use_litellm_proxy_by_default reads three switches: the environment, the global SDK setting and the deployment's own flag.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ProxySelection {
    pub environment: bool,
    pub global: bool,
}

// The LiteLLM_Params fields deployment attribution reads: the model, its declared provider and the per-deployment proxy flag.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DeploymentIdentity<'a> {
    pub model: &'a str,
    pub custom_llm_provider: Option<&'a str>,
    pub use_litellm_proxy: bool,
}

impl<'a> DeploymentIdentity<'a> {
    #[must_use]
    pub fn from_litellm_params(params: &'a JsonObject) -> Self {
        let model = match params.get("model") {
            Some(Json::Text(text)) => text.as_str(),
            Some(
                Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_),
            )
            | None => "",
        };
        let custom_llm_provider = match params.get("custom_llm_provider") {
            Some(Json::Text(text)) => Some(text.as_str()),
            Some(
                Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_),
            )
            | None => None,
        };
        Self {
            model,
            custom_llm_provider,
            use_litellm_proxy: matches!(params.get("use_litellm_proxy"), Some(Json::Bool(true))),
        }
    }
}

#[must_use]
pub fn should_use_litellm_proxy_by_default(
    selection: &ProxySelection,
    deployment_use_litellm_proxy: bool,
) -> bool {
    selection.environment || deployment_use_litellm_proxy || selection.global
}

// _deployment_provider: the proxy flag first, then B209's declared provider resolution, then fallback inference.
#[must_use]
pub fn deployment_provider(
    deployment: &DeploymentIdentity<'_>,
    selection: &ProxySelection,
    resolution: &ResolutionContext<'_>,
) -> Option<String> {
    if should_use_litellm_proxy_by_default(selection, deployment.use_litellm_proxy) {
        return Some(LITELLM_PROXY_PROVIDER.to_owned());
    }
    let custom_llm_provider = deployment
        .custom_llm_provider
        .filter(|provider| !provider.is_empty());
    if let Some(declared) = authenticating_provider(Some(deployment.model), custom_llm_provider) {
        return Some(<&'static str>::from(declared).to_owned());
    }
    let request = ResolutionRequest {
        model: deployment.model,
        custom_llm_provider,
        api_base: None,
        api_key: None,
    };
    resolve(&request, resolution)
        .ok()
        .map(|resolved| resolved.provider.slug().to_owned())
}

mod tests;
