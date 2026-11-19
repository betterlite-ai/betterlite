use super::deployment_info::get_model_info;
use super::discovery::{CatalogSnapshot, copy_configured, get_discovered_model_info};
use super::error::RouterMetadataError;
use super::metadata::{ModelMetadataSource, RouterMetadata};
use crate::model_catalog_lookup::fields::text;
use betterlite_models::decisions::Json;
use betterlite_models::model_catalog_keys::ModelCostKeys;
use betterlite_models::model_registration::{JsonObject, ModelRegistry};
use betterlite_models::resolution::{ResolutionContext, ResolutionRequest, ResolvedModel, resolve};
use secrecy::SecretString;
use std::error::Error;
const AZURE_ADVICE: &str =
    "https://docs.litellm.ai/docs/proxy/cost_tracking#spend-tracking-for-azure-openai-models";
#[derive(Clone, Copy, Debug)]
pub struct SuppliedDeployment<'row> {
    pub model_info: &'row JsonObject,
    pub litellm_params: Option<&'row JsonObject>,
}
struct MergedInputs<'row> {
    deployment: SuppliedDeployment<'row>,
    resolved: ResolvedModel,
    base_model: Option<String>,
}
pub async fn get_router_model_info<'row, Source>(
    metadata: &'row RouterMetadata<'_, '_, '_, Source>,
    deployment: Option<SuppliedDeployment<'row>>,
    received_model_name: &str,
    id: Option<&str>,
    keys: &mut ModelCostKeys,
) -> Result<Option<JsonObject>, RouterMetadataError<Source::Error>>
where
    Source: ModelMetadataSource,
{
    let inputs = resolve_inputs(metadata, deployment, id)?;
    let model = lookup_model(
        metadata.snapshot,
        metadata.registry,
        &inputs,
        received_model_name,
    );
    let looked_up = metadata
        .source
        .get_model_info(&model, keys)
        .await
        .map_err(|source| RouterMetadataError::Source { source })?;
    let Some(mut info) = looked_up else {
        return Ok(None);
    };
    let discovered = get_discovered_model_info(
        metadata.snapshot,
        metadata.discovery,
        text(inputs.deployment.model_info, "id"),
    );
    info.extend(discovered);
    copy_configured(&mut info, inputs.deployment.model_info);
    Ok(Some(info))
}
fn resolve_inputs<'row, Source, SourceError>(
    metadata: &'row RouterMetadata<'_, '_, '_, Source>,
    deployment: Option<SuppliedDeployment<'row>>,
    id: Option<&str>,
) -> Result<MergedInputs<'row>, RouterMetadataError<SourceError>>
where
    Source: ModelMetadataSource,
    SourceError: Error + 'static,
{
    let chosen = match id {
        Some(model_id) => match get_model_info(metadata.snapshot, model_id)? {
            Some(spec) => Some(SuppliedDeployment {
                model_info: &spec.model_info,
                litellm_params: Some(&spec.litellm_params),
            }),
            None => deployment,
        },
        None => deployment,
    };
    let Some(resolved_deployment) = chosen else {
        return Err(RouterMetadataError::DeploymentNotFound);
    };
    let params = checked_params(resolved_deployment)?;
    let configured = field_text(resolved_deployment.model_info, "base_model")?;
    let base_model = match configured {
        Some(base_model) => Some(base_model.to_owned()),
        None => field_text(params, "base_model")?.map(str::to_owned),
    };
    let resolved = resolve_params(params, metadata.resolution)?;
    Ok(MergedInputs {
        deployment: resolved_deployment,
        resolved,
        base_model,
    })
}
fn checked_params<'row, SourceError>(
    deployment: SuppliedDeployment<'row>,
) -> Result<&'row JsonObject, RouterMetadataError<SourceError>>
where
    SourceError: Error + 'static,
{
    match deployment.litellm_params {
        Some(params) if params.contains_key("model") => Ok(params),
        Some(_params) => Err(RouterMetadataError::InvalidParameters {
            params_type: "dict",
            deployment_id: text(deployment.model_info, "id")
                .unwrap_or("unknown")
                .to_owned(),
        }),
        None => Err(RouterMetadataError::InvalidParameters {
            params_type: "NoneType",
            deployment_id: text(deployment.model_info, "id")
                .unwrap_or("unknown")
                .to_owned(),
        }),
    }
}
fn field_text<'params, SourceError>(
    params: &'params JsonObject,
    field: &'static str,
) -> Result<Option<&'params str>, RouterMetadataError<SourceError>>
where
    SourceError: Error + 'static,
{
    match params.get(field) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(value)) => Ok(Some(value)),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(RouterMetadataError::FieldNotText { field })
        }
    }
}
fn resolve_params<SourceError>(
    params: &JsonObject,
    resolution: &ResolutionContext<'_>,
) -> Result<ResolvedModel, RouterMetadataError<SourceError>>
where
    SourceError: Error + 'static,
{
    let Some(model) = field_text(params, "model")? else {
        return Err(RouterMetadataError::FieldNotText { field: "model" });
    };
    let custom_llm_provider = field_text(params, "custom_llm_provider")?;
    let api_base = field_text(params, "api_base")?;
    let api_key = field_text(params, "api_key")?.map(SecretString::from);
    let request = ResolutionRequest {
        model,
        custom_llm_provider,
        api_base,
        api_key: api_key.as_ref(),
    };
    Ok(resolve(&request, resolution)?)
}
fn lookup_model(
    snapshot: &CatalogSnapshot<'_>,
    registry: &ModelRegistry,
    inputs: &MergedInputs<'_>,
    received_model_name: &str,
) -> String {
    let provider = inputs.resolved.provider.slug();
    if provider != "azure" {
        let mut model = inputs.resolved.model.clone();
        if model.contains('*') {
            model =
                wildcard_model(snapshot, inputs.deployment, received_model_name).unwrap_or(model);
        }
        return prefixed(provider, &model);
    }
    if inputs.base_model.is_none() {
        log_azure_fallback(&inputs.resolved, registry);
    }
    let model = inputs
        .base_model
        .clone()
        .unwrap_or_else(|| inputs.resolved.model.clone());
    prefixed(provider, &model)
}
fn prefixed(provider: &str, model: &str) -> String {
    if model.starts_with(&format!("{provider}/")) {
        return model.to_owned();
    }
    format!("{provider}/{model}")
}
fn wildcard_model(
    snapshot: &CatalogSnapshot<'_>,
    deployment: SuppliedDeployment<'_>,
    received_model_name: &str,
) -> Option<String> {
    let id = text(deployment.model_info, "id");
    snapshot
        .catalog
        .pattern_router
        .route(received_model_name, None)?
        .into_iter()
        .find(|candidate| candidate.model_id.as_deref() == id)
        .filter(|candidate| !candidate.model.is_empty())
        .map(|candidate| candidate.model)
}
fn positive(value: Option<&Json>) -> bool {
    match value {
        None | Some(Json::Null) => false,
        Some(Json::Bool(flag)) => *flag,
        Some(Json::Number(number)) => number.as_f64().is_some_and(|amount| amount > 0.0),
        Some(Json::Text(_) | Json::List(_) | Json::Object(_)) => false,
    }
}
fn log_azure_fallback(resolved: &ResolvedModel, registry: &ModelRegistry) {
    let model = resolved.model.as_str();
    let fallback_key = if model.starts_with("azure/") {
        model.to_owned()
    } else {
        format!("azure/{model}")
    };
    let usable = match registry.model_cost.get(&fallback_key) {
        Some(Json::Object(fields)) => ["max_input_tokens", "max_tokens", "input_cost_per_token"]
            .into_iter()
            .any(|field| positive(fields.get(field))),
        None
        | Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            false
        }
    };
    if usable {
        tracing::debug!(
            "Azure deployment '{model}' has no base_model set; using '{fallback_key}' from the model cost map for max tokens, cost tracking, etc."
        );
    } else {
        tracing::error!(
            "Could not identify azure model '{model}'. Set azure 'base_model' for accurate max tokens, cost tracking, etc.- {AZURE_ADVICE}"
        );
    }
}
