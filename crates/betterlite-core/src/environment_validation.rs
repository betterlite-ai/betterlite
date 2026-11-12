use std::collections::BTreeSet;

use betterlite_models::provider_model_sets::ProviderModelSets;
use secrecy::SecretString;

use crate::cost::context::ProviderInference;

pub mod config_args;
pub mod error;
pub mod probe;
pub mod provider_fields;
mod rows;
mod tests;

pub struct EnvironmentRequest<'request> {
    pub model: Option<&'request str>,
    pub api_key: Option<&'request SecretString>,
    pub api_base: Option<&'request str>,
    pub api_version: Option<&'request str>,
}

#[derive(Debug, Eq, PartialEq)]
pub struct EnvironmentCheck {
    pub keys_in_environment: bool,
    pub missing_keys: Vec<&'static str>,
}

// Only names are read, never values; a name set to an empty string still counts as present.
pub fn validate_environment<Inference>(
    request: &EnvironmentRequest<'_>,
    environment_names: &BTreeSet<String>,
    model_sets: &ProviderModelSets,
    inference: &Inference,
) -> EnvironmentCheck
where
    Inference: ProviderInference,
{
    let Some(model) = request.model else {
        return EnvironmentCheck {
            keys_in_environment: false,
            missing_keys: Vec::new(),
        };
    };
    // A failing provider lookup sends the model through the model-set chain instead.
    let row = inference.inferred_provider(model).map_or_else(
        |_| rows::model_chain_row(model_sets, model),
        |provider| rows::provider_row(provider.slug()),
    );
    let mut missing_keys =
        row.map_or_else(Vec::new, |required| required.missing(environment_names));
    let passed = [
        (request.api_key.is_some(), "api_key"),
        (request.api_base.is_some(), "api_base"),
        (request.api_version.is_some(), "api_version"),
    ];
    for (_, pattern) in passed.iter().filter(|(given, _)| *given) {
        missing_keys.retain(|key| !key.to_ascii_lowercase().contains(pattern));
    }
    EnvironmentCheck {
        keys_in_environment: missing_keys.is_empty(),
        missing_keys,
    }
}
