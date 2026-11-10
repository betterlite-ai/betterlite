use std::collections::{BTreeMap, BTreeSet};

use betterlite_models::catalog::{ModelCatalog, UnknownFields};
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::{ResolutionContext, ResolutionRequest, resolve};
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::json;
use serde_json::value::RawValue;

use crate::helpers::{Helper, HelperError, HelperResult};

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    model: String,
    #[serde(default)]
    custom_llm_provider: Option<String>,
    #[serde(default)]
    api_base: Option<String>,
    #[serde(default)]
    api_key: Option<String>,
    #[serde(default)]
    environment: BTreeMap<String, String>,
    #[serde(default)]
    known_model_names: Vec<String>,
    #[serde(default)]
    custom_providers: Vec<String>,
    #[serde(default)]
    catalog: Option<Box<RawValue>>,
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let catalog = parse_catalog(input.catalog.as_deref())?;
        let known: BTreeSet<String> = input.known_model_names.iter().cloned().collect();
        let custom: BTreeSet<String> = input.custom_providers.iter().cloned().collect();
        let context = ResolutionContext {
            catalog: &catalog,
            known_model_names: &known,
            custom_providers: &custom,
            environment: &input.environment,
            registry: JSONProviderRegistry::load(),
            policies: ProviderPolicies::default(),
            globals: GlobalCredentials::default(),
        };
        resolve_one(input, &context)
    }
}

fn parse_catalog(raw: Option<&RawValue>) -> Result<ModelCatalog, Box<HelperError>> {
    let Some(source) = raw else {
        return Ok(ModelCatalog::default());
    };
    ModelCatalog::parse(source.get().as_bytes(), UnknownFields::Reject).map_err(|error| {
        Box::new(HelperError {
            class: String::from("InvalidInput"),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: error.to_string(),
        })
    })
}

fn resolve_one(input: &Input, context: &ResolutionContext<'_>) -> HelperResult {
    let api_key = input.api_key.clone().map(SecretString::from);
    let request = ResolutionRequest {
        model: &input.model,
        custom_llm_provider: input.custom_llm_provider.as_deref(),
        api_base: input.api_base.as_deref(),
        api_key: api_key.as_ref(),
    };
    let resolved = resolve(&request, context).map_err(|error| {
        Box::new(HelperError {
            class: String::from("BadRequestError"),
            status: Some(400),
            error_type: None,
            param: None,
            code: None,
            message: error.to_string(),
        })
    })?;
    Ok(json!({
        "model": resolved.model,
        "provider": resolved.provider.slug(),
        "api_base": resolved.api_base,
        "api_key": resolved.api_key.as_ref().map(secrecy::ExposeSecret::expose_secret),
    }))
}
