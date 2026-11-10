use betterlite_models::catalog::BundledCatalog;
use betterlite_models::endpoints::projection::project;
use betterlite_models::endpoints::{
    EndpointRegistry, EndpointSupport, PACKAGED_ASSET, RUNTIME_ASSET, RegistryAsset, parse, support,
};
use betterlite_models::lookup::capabilities::CapabilityView;
use betterlite_models::lookup::info::{ModelInfoError, ModelQuery, declared_provider, find_entry};
use betterlite_models::lookup::limits::{ModelLimits, max_output_tokens};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::helpers::{HelperError, HelperResult};

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    operation: Operation,
    #[serde(default)]
    source: Option<Value>,
    #[serde(default)]
    queries: Vec<ModelQueryInput>,
    #[serde(default)]
    models: Vec<String>,
    #[serde(default)]
    routes: Vec<RouteQueryInput>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum Operation {
    RootRegistry,
    PackagedRegistry,
    ModelInfo,
    MaxTokens,
    ProviderEndpoint,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ModelQueryInput {
    model: String,
    #[serde(default)]
    provider: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct RouteQueryInput {
    provider: String,
    endpoint: String,
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Input) -> HelperResult {
        match input.operation {
            Operation::RootRegistry => {
                let registry = root_registry(input)?;
                Ok(registry_projection(&registry))
            }
            Operation::PackagedRegistry => {
                let registry = parse(PACKAGED_ASSET, RUNTIME_ASSET)
                    .map_err(|error| helper_error(error.to_string()))?;
                Ok(registry_projection(&registry))
            }
            Operation::ModelInfo => model_infos(input),
            Operation::MaxTokens => max_token_limits(input),
            Operation::ProviderEndpoint => route_queries(input),
        }
    }
}

fn root_registry(input: &Input) -> Result<EndpointRegistry, Box<HelperError>> {
    let source = input
        .source
        .as_ref()
        .ok_or_else(|| helper_error("root_registry requires a source".to_owned()))?;
    let bytes = serde_json::to_vec(source).map_err(|error| helper_error(error.to_string()))?;
    parse(&bytes, RegistryAsset::Root).map_err(|error| helper_error(error.to_string()))
}

fn registry_projection(registry: &EndpointRegistry) -> Value {
    let endpoints = project(registry);
    json!({
        "provider_count": registry.providers.len(),
        "endpoints": endpoints,
    })
}

fn model_infos(input: &Input) -> HelperResult {
    let catalog = BundledCatalog::default().get();
    let mut results = Vec::with_capacity(input.queries.len());
    for query in &input.queries {
        let typed = ModelQuery {
            model: &query.model,
            provider: query.provider.as_deref(),
        };
        let Some(entry) = find_entry(&catalog, &typed) else {
            return Err(helper_error(ModelInfoError::unmapped(&typed).to_string()));
        };
        let capabilities = CapabilityView::from_fields(&entry.fields.capabilities);
        let limits = ModelLimits::from_fields(&entry.fields.metadata);
        results.push(json!({
            "key": entry.name,
            "provider": declared_provider(entry, &typed),
            "mode": entry.fields.metadata.mode,
            "max_tokens": limits.max_tokens,
            "max_input_tokens": limits.max_input_tokens,
            "max_output_tokens": limits.max_output_tokens,
            "supports_vision": capabilities.supports_vision,
            "supports_function_calling": capabilities.supports_function_calling,
            "supports_tool_choice": capabilities.supports_tool_choice,
            "supports_response_schema": capabilities.supports_response_schema,
            "supports_prompt_caching": capabilities.supports_prompt_caching,
            "supports_reasoning": capabilities.supports_reasoning,
            "supports_web_search": capabilities.supports_web_search,
            "supports_pdf_input": capabilities.supports_pdf_input,
        }));
    }
    Ok(json!({"results": results}))
}

fn max_token_limits(input: &Input) -> HelperResult {
    let catalog = BundledCatalog::default().get();
    let mut results = Vec::with_capacity(input.models.len());
    for model in &input.models {
        let max_tokens =
            max_output_tokens(&catalog, model).map_err(|error| helper_error(error.to_string()))?;
        results.push(json!({"model": model, "max_tokens": max_tokens}));
    }
    Ok(json!({"results": results}))
}

fn route_queries(input: &Input) -> HelperResult {
    let registry =
        parse(PACKAGED_ASSET, RUNTIME_ASSET).map_err(|error| helper_error(error.to_string()))?;
    let projection = project(&registry);
    let mut results = Vec::with_capacity(input.routes.len());
    for route in &input.routes {
        let catalog_support = match support(&registry, &route.provider, &route.endpoint) {
            EndpointSupport::Supported => Value::Bool(true),
            EndpointSupport::Unsupported => Value::Bool(false),
            EndpointSupport::Unlisted => Value::Null,
        };
        let supporting: Vec<&str> = projection
            .iter()
            .find(|entry| entry.key == route.endpoint)
            .map(|entry| {
                entry
                    .providers
                    .iter()
                    .map(|provider| provider.slug.as_str())
                    .collect()
            })
            .unwrap_or_default();
        results.push(json!({
            "provider": route.provider,
            "endpoint": route.endpoint,
            "support": catalog_support,
            "providers": supporting,
        }));
    }
    Ok(json!({"results": results}))
}

fn helper_error(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: "InvalidInput".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

mod tests;
