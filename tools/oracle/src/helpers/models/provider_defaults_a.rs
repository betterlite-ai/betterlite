use std::collections::BTreeMap;

use betterlite_bedrock::mantle_inputs::chat_endpoint::{ChatEndpointParams, chat_endpoint_inputs};
use betterlite_bedrock::mantle_inputs::records::MantleGlobals;
use betterlite_bedrock::mantle_inputs::region::split_region_prefix;
use betterlite_bedrock::model_routes::{ParamMap, ParamValue};
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::provider_defaults_a::error::ProviderDefaultsError;
use betterlite_models::provider_defaults_a::model::{DefaultsProvider, ProviderDefaultsRequest};
use betterlite_models::provider_defaults_a::policies::{
    MantleEndpoint, ModelSets, ProviderPolicies,
};
use betterlite_models::provider_defaults_a::resolve::resolve_provider_defaults;
use indexmap::IndexMap;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::json;

use crate::helpers::{Helper, HelperError, HelperResult};

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    provider: String,
    model: String,
    #[serde(default)]
    api_base: Option<String>,
    #[serde(default)]
    api_key: Option<String>,
    #[serde(default)]
    environment: BTreeMap<String, String>,
    #[serde(default)]
    openai_chat_models: Vec<String>,
    #[serde(default)]
    openai_text_completion_models: Vec<String>,
    #[serde(default)]
    ai21_chat_models: Vec<String>,
    #[serde(default)]
    mantle_openai_path_models: Vec<String>,
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let provider = provider_from_slug(&input.provider)?;
        let api_key = input.api_key.clone().map(SecretString::from);
        let policies = ProviderPolicies {
            models: ModelSets {
                openai_chat: input.openai_chat_models.iter().cloned().collect(),
                openai_text_completion: input
                    .openai_text_completion_models
                    .iter()
                    .cloned()
                    .collect(),
                ai21_chat: input.ai21_chat_models.iter().cloned().collect(),
            },
            mantle: mantle_endpoint(input, provider, api_key.as_ref())?,
        };
        resolve(input, provider, api_key, &policies)
    }
}

fn resolve(
    input: &Input,
    provider: DefaultsProvider,
    api_key: Option<SecretString>,
    policies: &ProviderPolicies,
) -> HelperResult {
    let request = ProviderDefaultsRequest {
        provider,
        model: input.model.clone(),
        api_base: input.api_base.clone(),
        api_key,
    };
    let registry = JSONProviderRegistry::load();
    let resolved = resolve_provider_defaults(request, &input.environment, &registry, policies)
        .map_err(|error| Box::new(resolver_error(&error)))?;
    Ok(json!({
        "provider": resolved.provider.slug(),
        "model": resolved.model,
        "api_base": resolved.api_base,
        "api_key": resolved.api_key.as_ref().map(secrecy::ExposeSecret::expose_secret),
    }))
}

fn provider_from_slug(slug: &str) -> Result<DefaultsProvider, Box<HelperError>> {
    if slug == "anyscale" {
        return Ok(DefaultsProvider::Anyscale);
    }
    slug.parse::<ProviderId>()
        .map(DefaultsProvider::Known)
        .map_err(|error| {
            Box::new(helper_error(
                "InvalidInput",
                format!("unknown provider slug: {error}"),
            ))
        })
}

fn mantle_endpoint(
    input: &Input,
    provider: DefaultsProvider,
    api_key: Option<&SecretString>,
) -> Result<Option<MantleEndpoint>, Box<HelperError>> {
    if provider != DefaultsProvider::Known(ProviderId::BedrockMantle) {
        return Ok(None);
    }
    let params = ChatEndpointParams {
        model: Some(&input.model),
        api_base: input.api_base.as_deref(),
        api_key,
        aws_region_name: None,
    };
    let endpoint = chat_endpoint_inputs(params, &mantle_globals(input), &model_cost(input))
        .map_err(|error| Box::new(helper_error("MantleInputError", error.to_string())))?;
    let (_, base_model) = split_region_prefix(&input.model);
    Ok(Some(MantleEndpoint {
        api_base: endpoint.api_base,
        api_key: endpoint.api_key,
        base_model: base_model.to_owned(),
    }))
}

fn mantle_globals(input: &Input) -> MantleGlobals {
    let environment = &input.environment;
    MantleGlobals {
        mantle_api_key: environment
            .get("BEDROCK_MANTLE_API_KEY")
            .cloned()
            .map(SecretString::from),
        mantle_api_base: environment.get("BEDROCK_MANTLE_API_BASE").cloned(),
        mantle_region: environment.get("BEDROCK_MANTLE_REGION").cloned(),
        aws_bearer_token: environment
            .get("AWS_BEARER_TOKEN_BEDROCK")
            .cloned()
            .map(SecretString::from),
        aws_region_name: environment.get("AWS_REGION_NAME").cloned(),
        aws_region: environment.get("AWS_REGION").cloned(),
    }
}

fn model_cost(input: &Input) -> ParamMap {
    let mut model_cost = ParamMap::new();
    for name in &input.mantle_openai_path_models {
        let mut fields = IndexMap::new();
        fields.insert(
            "use_openai_responses_path".to_owned(),
            ParamValue::Bool(true),
        );
        model_cost.insert(
            format!("bedrock_mantle/{name}"),
            ParamValue::Mapping(fields),
        );
    }
    model_cost
}

fn resolver_error(error: &ProviderDefaultsError) -> HelperError {
    let class = match error {
        ProviderDefaultsError::UnknownProvider { .. } => "UnknownProvider",
        ProviderDefaultsError::MissingMantleEndpoint => "MissingMantleEndpoint",
        ProviderDefaultsError::InvalidApiBase { .. } => "ValueError",
    };
    helper_error(class, error.to_string())
}

fn helper_error(class: &str, message: String) -> HelperError {
    HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    }
}
