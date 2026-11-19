use std::error::Error;

use betterlite_models::decisions::Json;
use betterlite_models::model_catalog_keys::ModelCostKeys;
use betterlite_models::model_registration::{JsonObject, ModelRegistry};
use betterlite_models::resolution::{ResolutionContext, ResolutionRequest, ResolvedModel, resolve};
use secrecy::SecretString;

use crate::deployment_identity::spec::DeploymentSpec;
use crate::model_catalog_lookup::fields::text;

use super::deployment_info::get_model_info;
use super::discovery::{copy_configured, get_discovered_model_info};
use super::error::RouterMetadataError;
use super::metadata::{ModelMetadataSource, RouterMetadata};

const AZURE_ADVICE: &str =
    "https://docs.litellm.ai/docs/proxy/cost_tracking#spend-tracking-for-azure-openai-models";

// A deployment the caller supplied: the model_info and litellm_params raw enough for the merge to read.
#[derive(Clone, Copy, Debug)]
pub struct SuppliedDeployment<'row> {
    pub model_info: &'row JsonObject,
    pub litellm_params: Option<&'row JsonObject>,
}

impl<'row> SuppliedDeployment<'row> {
    #[must_use]
    pub fn from_spec(spec: &'row DeploymentSpec) -> Self {
        Self {
            model_info: &spec.model_info,
            litellm_params: Some(&spec.litellm_params),
        }
    }
}

// Router.get_router_model_info: the deployment's cost-map entry, discovery and configured model_info merged.
pub async fn get_router_model_info<Source>(
    metadata: &RouterMetadata<'_, '_, '_, Source>,
    deployment: Option<SuppliedDeployment<'_>>,
    received_model_name: &str,
    id: Option<&str>,
    keys: &mut ModelCostKeys,
) -> Result<Option<JsonObject>, RouterMetadataError<Source::Error>>
where
    Source: ModelMetadataSource,
{
    let supplied = resolve_inputs(metadata, deployment, id)?;
    let (deployment, params, resolved) = supplied;
    let base_model = base_model_text(deployment.model_info, params)?;
    let model = lookup_model(
        metadata.snapshot,
        metadata.registry,
        resolved,
        base_model,
        deployment,
        received_model_name,
    );
    let Some(mut info) = source_info(metadata, &model, keys).await? else {
        return Ok(None);
    };
    for (key, value) in get_discovered_model_info(
        metadata.snapshot,
        metadata.discovery,
        text(deployment.model_info, "id"),
    ) {
        info.insert(key, value);
    }
    copy_configured(&mut info, deployment.model_info);
    Ok(Some(info))
}

type ResolvedInputs<'row> = (SuppliedDeployment<'row>, &'row JsonObject, ResolvedModel);

// The id override, the missing-deployment error, the params check and B174's resolution.
fn resolve_inputs<'row, SourceError>(
    metadata: &RouterMetadata<'_, '_, '_, impl ModelMetadataSource>,
    deployment: Option<SuppliedDeployment<'row>>,
    id: Option<&str>,
) -> Result<ResolvedInputs<'row>, RouterMetadataError<SourceError>>
where
    SourceError: Error + 'static,
{
    let deployment = match id {
        Some(model_id) => match get_model_info(metadata.snapshot, model_id)? {
            Some(spec) => Some(SuppliedDeployment::from_spec(spec)),
            None => deployment,
        },
        None => deployment,
    };
    let Some(deployment) = deployment else {
        return Err(RouterMetadataError::DeploymentNotFound);
    };
    let params = checked_params(deployment)?;
    let resolved = resolve_params(params, metadata.resolution)?;
    Ok((deployment, params, resolved))
}

fn deployment_id(model_info: &JsonObject) -> String {
    text(model_info, "id").unwrap_or("unknown").to_owned()
}

// deployment.get("litellm_params") as LiteLLM_Params sees it: an object with a model, or the ValueError.
fn checked_params<'row, SourceError>(
    deployment: SuppliedDeployment<'row>,
) -> Result<&'row JsonObject, RouterMetadataError<SourceError>>
where
    SourceError: Error + 'static,
{
    match deployment.litellm_params {
        Some(params) if params.contains_key("model") => Ok(params),
        Some(_) => Err(RouterMetadataError::InvalidParameters {
            params_type: "dict",
            deployment_id: deployment_id(deployment.model_info),
        }),
        None => Err(RouterMetadataError::InvalidParameters {
            params_type: "NoneType",
            deployment_id: deployment_id(deployment.model_info),
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
    let model = match field_text(params, "model")? {
        Some(model) => model,
        None => return Err(RouterMetadataError::FieldNotText { field: "model" }),
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

// model_info.base_model unless null, then litellm_params.base_model, as get_router_model_info reads the pair.
fn base_model_text<SourceError>(
    model_info: &JsonObject,
    params: &JsonObject,
) -> Result<Option<String>, RouterMetadataError<SourceError>>
where
    SourceError: Error + 'static,
{
    let configured = field_text(model_info, "base_model")?;
    match configured {
        Some(base_model) => Ok(Some(base_model.to_owned())),
        None => Ok(field_text(params, "base_model")?.map(str::to_owned)),
    }
}

// The cost-map name the source receives: the resolved model for every provider but azure, and the provider prefix once.
fn lookup_model<SourceError>(
    snapshot: &CatalogSnapshot<'_>,
    registry: &ModelRegistry,
    resolved: ResolvedModel,
    base_model: Option<String>,
    deployment: SuppliedDeployment<'_>,
    received_model_name: &str,
) -> Result<String, RouterMetadataError<SourceError>>
where
    SourceError: Error + 'static,
{
    let provider = resolved.provider.slug();
    let mut model = base_model;
    if provider == "azure" {
        if model.is_none() {
            log_azure_fallback(&resolved, registry);
        }
    } else {
        model = Some(resolved.model.clone());
        if resolved.model.contains('*') {
            model = wildcard_model(snapshot, deployment, received_model_name).or(model);
        }
    }
    let model = model.unwrap_or_else(|| resolved.model.clone());
    if model.starts_with(&format!("{provider}/")) {
        return Ok(model);
    }
    Ok(format!("{provider}/{model}"))
}

// The first expansion of received_model_name that carries this deployment's id, silently skipping the rest.
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

// The azure deployment without a base_model gets a debug hint or the error that names the advice.
fn log_azure_fallback(resolved: &ResolvedModel, registry: &ModelRegistry) {
    let model = resolved.model.as_str();
    let fallback_key = match model.starts_with("azure/") {
        true => model.to_owned(),
        false => format!("azure/{model}"),
    };
    let entry = match registry.model_cost.get(&fallback_key) {
        Some(Json::Object(fields)) => Some(fields),
        None
        | Some(
            Json::Null
            | Json::Bool(_)
            | Json::Number(_)
            | Json::Text(_)
            | Json::List(_)
            | Json::Object(_),
        ) => None,
    };
    let usable = entry.is_some_and(|fields| {
        ["max_input_tokens", "max_tokens", "input_cost_per_token"]
            .into_iter()
            .any(|field| positive(fields.get(field)))
    });
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

// The source's answer, its failure propagated with the lookup name.
async fn source_info<Source>(
    metadata: &RouterMetadata<'_, '_, '_, Source>,
    model: &str,
    keys: &mut ModelCostKeys,
) -> Result<Option<JsonObject>, RouterMetadataError<Source::Error>>
where
    Source: ModelMetadataSource,
{
    metadata
        .source
        .get_model_info(model, keys)
        .await
        .map_err(|source| RouterMetadataError::Source { source })
}
