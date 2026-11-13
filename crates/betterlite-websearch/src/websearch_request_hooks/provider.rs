use std::collections::BTreeSet;

use betterlite_models::catalog::ModelCatalog;
use betterlite_models::identity::explicit_prefix::{
    PrefixClassification, PrefixClassificationInput, classify_prefix,
};
use betterlite_prompts::arguments::json::{Json, Object};
use thiserror::Error;

const LITELLM_PARAMS: &str = "litellm_params";
const MODEL: &str = "model";
const PROVIDER: &str = "custom_llm_provider";

#[derive(Debug, Error)]
pub enum ProviderError {
    #[error("litellm_params must be an object")]
    LitellmParams,
    #[error("custom_llm_provider must be a string")]
    CustomProvider,
}

pub fn deployment_provider(kwargs: &Object) -> Result<Option<String>, ProviderError> {
    if let Some(provider) = provider_field(kwargs.get(PROVIDER))? {
        return Ok(Some(provider.to_owned()));
    }
    if let Some(provider) = nested_provider(kwargs)? {
        return Ok(Some(provider.to_owned()));
    }
    Ok(model_provider(kwargs))
}

pub fn request_provider(kwargs: &Object) -> Result<Option<String>, ProviderError> {
    let Some(provider) = nested_provider(kwargs)? else {
        return Ok(None);
    };
    Ok(Some(provider.to_owned()))
}

fn nested_provider(kwargs: &Object) -> Result<Option<&str>, ProviderError> {
    match kwargs.get(LITELLM_PARAMS) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Object(params)) => provider_field(params.get(PROVIDER)),
        Some(_) => Err(ProviderError::LitellmParams),
    }
}

fn provider_field(value: Option<&Json>) -> Result<Option<&str>, ProviderError> {
    match value {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(text)) if text.is_empty() => Ok(None),
        Some(Json::Text(text)) => Ok(Some(text.as_str())),
        Some(_) => Err(ProviderError::CustomProvider),
    }
}

// get_llm_provider also infers from the cost map; a hook has no catalog and resolves the declared prefix
fn model_provider(kwargs: &Object) -> Option<String> {
    let model = kwargs.get(MODEL).and_then(Json::as_str)?;
    let catalog = ModelCatalog::default();
    let no_names = BTreeSet::new();
    let classification = classify_prefix(&PrefixClassificationInput {
        model,
        custom_llm_provider: None,
        configured_model_names: &no_names,
        catalog: &catalog,
    });
    match classification {
        PrefixClassification::ExplicitProvider { provider, .. } => {
            Some(<&'static str>::from(provider).to_owned())
        }
        PrefixClassification::NonOpenaiAzure { .. }
        | PrefixClassification::OpenRouterNative { .. }
        | PrefixClassification::OpenRouterNested { .. }
        | PrefixClassification::Unresolved { .. } => None,
    }
}
