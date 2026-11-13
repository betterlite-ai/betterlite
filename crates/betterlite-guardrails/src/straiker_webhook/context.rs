use std::collections::{BTreeMap, BTreeSet};

use betterlite_models::catalog::ModelCatalog;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_a::url::parse as parse_urlsplit;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::{ResolutionContext, ResolutionRequest, resolve};
use betterlite_prompts::arguments::json::{Json, Object};
use secrecy::SecretString;

const UNKNOWN_CALL_SURFACE: &str = "unknown";

pub trait ProviderNames {
    fn provider_for(
        &self,
        model: &str,
        api_base: Option<&str>,
        api_key: Option<&str>,
    ) -> Option<String>;
}

// The caller owns the resolver's tables; the credential globals never change the provider slug reported here.
pub struct CatalogProviderNames<'context> {
    pub catalog: &'context ModelCatalog,
    pub known_model_names: &'context BTreeSet<String>,
    pub custom_providers: &'context BTreeSet<String>,
    pub environment: &'context BTreeMap<String, String>,
    pub registry: &'context JSONProviderRegistry,
}

impl ProviderNames for CatalogProviderNames<'_> {
    fn provider_for(
        &self,
        model: &str,
        api_base: Option<&str>,
        api_key: Option<&str>,
    ) -> Option<String> {
        let secret = api_key.map(str::to_owned).map(SecretString::from);
        let request = ResolutionRequest {
            model,
            custom_llm_provider: None,
            api_base,
            api_key: secret.as_ref(),
        };
        let context = ResolutionContext {
            catalog: self.catalog,
            known_model_names: self.known_model_names,
            custom_providers: self.custom_providers,
            environment: self.environment,
            registry: self.registry.clone(),
            policies: ProviderPolicies::default(),
            globals: GlobalCredentials::default(),
        };
        resolve(&request, &context)
            .ok()
            .map(|resolved| resolved.provider.slug().to_owned())
    }
}

#[must_use]
pub fn model_provider(
    request_data: &Object,
    model: Option<&str>,
    names: &dyn ProviderNames,
) -> Option<String> {
    let litellm_params = request_data.get("litellm_params").and_then(Json::as_object);
    let declared = merged_text(request_data, litellm_params, "custom_llm_provider");
    if let Some(provider) = declared {
        return Some(provider.to_owned());
    }
    let model_name = model?;
    let api_base = merged_text(request_data, litellm_params, "api_base");
    let api_key = merged_text(request_data, litellm_params, "api_key");
    names.provider_for(model_name, api_base, api_key)
}

#[must_use]
pub fn destination_host(request_data: &Object) -> Option<String> {
    let litellm_params = request_data.get("litellm_params").and_then(Json::as_object);
    let api_base = merged_text(request_data, litellm_params, "api_base")?;
    parse_urlsplit(api_base).ok()?.hostname()
}

#[must_use]
pub fn call_surface(logging_call_type: Option<&str>, request_data: &Object) -> String {
    let request_call_type = |key: &str| {
        request_data
            .get(key)
            .and_then(Json::as_str)
            .filter(|text| !text.is_empty())
    };
    let candidate = logging_call_type
        .filter(|text| !text.is_empty())
        .or_else(|| request_call_type("call_type"))
        .or_else(|| request_call_type("litellm_call_type"));
    candidate.unwrap_or(UNKNOWN_CALL_SURFACE).to_owned()
}

fn merged_text<'text>(
    request_data: &'text Object,
    params: Option<&'text Object>,
    key: &str,
) -> Option<&'text str> {
    request_data.get(key).and_then(Json::as_str).or_else(|| {
        params
            .and_then(|config| config.get(key))
            .and_then(Json::as_str)
    })
}

mod tests;
