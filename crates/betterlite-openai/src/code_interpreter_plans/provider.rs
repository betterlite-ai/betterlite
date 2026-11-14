use std::collections::{BTreeMap, BTreeSet};

use betterlite_models::catalog::ModelCatalog;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::{ResolutionContext, ResolutionRequest, resolve};
use serde_json::{Map, Value};

// LiteLLM's _resolve_provider: a declared provider wins, otherwise get_llm_provider reads the model string
#[must_use]
pub fn resolve_provider(catalog: &ModelCatalog, kwargs: &Map<String, Value>) -> Option<String> {
    if let Some(provider) = kwargs
        .get("custom_llm_provider")
        .and_then(Value::as_str)
        .filter(|provider| !provider.is_empty())
    {
        return Some(provider.to_owned());
    }
    let model = kwargs.get("model")?.as_str()?;
    let known_model_names = BTreeSet::new();
    let custom_providers = BTreeSet::new();
    let environment = BTreeMap::new();
    let context = ResolutionContext {
        catalog,
        known_model_names: &known_model_names,
        custom_providers: &custom_providers,
        environment: &environment,
        registry: JSONProviderRegistry::default(),
        policies: ProviderPolicies::default(),
        globals: GlobalCredentials::default(),
    };
    let request = ResolutionRequest {
        model,
        custom_llm_provider: None,
        api_base: None,
        api_key: None,
    };
    resolve(&request, &context)
        .ok()
        .map(|resolved| resolved.provider.slug().to_owned())
}

include!("provider/tests/cases.rs");
