use std::collections::BTreeMap;

use indexmap::IndexMap;
use serde_json::from_str;
use serde_json::value::RawValue;

use crate::decisions::Json;
use crate::identity::declared::authenticating_provider;
use crate::lemonade_model_metadata::ModelInfoClient;
use crate::model_catalog_keys::ModelCostKeys;
use crate::model_metadata_resolution::MetadataRequest;
use crate::model_metadata_resolution::dynamic::DynamicMetadataSource;
use crate::model_metadata_resolution::error::ModelMetadataError;
use crate::model_metadata_resolution::get_model_info_helper;
use crate::provider_metadata_factory::selection::{MetadataQuery, get_provider_info};
use crate::resolution::{ResolutionError, ResolutionRequest, resolve};

use super::{CapabilityEnvironment, CapabilityRequest};

// Python's dict.get: absent and null read the same, and any other value is compared by identity or read as text.
pub(super) enum MetadataValue {
    Absent,
    Value(Json),
}

// LiteLLM catches this whole body as one Exception; a resolution or metadata failure takes the same path.
#[derive(Debug, thiserror::Error)]
pub(super) enum CapabilityError {
    #[error(transparent)]
    Resolution(#[from] ResolutionError),
    #[error(transparent)]
    Metadata(#[from] ModelMetadataError),
    #[error("model metadata fields: {0}")]
    Fields(#[from] serde_json::Error),
}

pub(super) struct Target {
    pub(super) model: String,
    pub(super) provider: String,
}

// declared_authenticating_provider answers for the OAuth providers, whose prefix is the whole declaration.
pub(super) fn declared_target(request: &CapabilityRequest<'_>) -> Option<Target> {
    let declared = authenticating_provider(Some(request.model), request.custom_llm_provider)?;
    let provider = <&'static str>::from(declared).to_owned();
    let prefix = format!("{provider}/");
    let stripped = request
        .model
        .strip_prefix(prefix.as_str())
        .unwrap_or(request.model);
    Some(Target {
        model: stripped.to_owned(),
        provider,
    })
}

pub(super) fn resolve_target<Source, Transport>(
    request: &CapabilityRequest<'_>,
    environment: &CapabilityEnvironment<'_, Source, Transport>,
) -> Result<Target, ResolutionError> {
    if let Some(target) = declared_target(request) {
        return Ok(target);
    }
    resolved_target(request, environment)
}

// get_llm_provider without the authenticating fast path, which the string readers and streaming call.
pub(super) fn resolved_target<Source, Transport>(
    request: &CapabilityRequest<'_>,
    environment: &CapabilityEnvironment<'_, Source, Transport>,
) -> Result<Target, ResolutionError> {
    let resolution = ResolutionRequest {
        model: request.model,
        custom_llm_provider: request.custom_llm_provider,
        api_base: None,
        api_key: None,
    };
    let resolved = resolve(&resolution, environment.metadata.resolution)?;
    Ok(Target {
        model: resolved.model,
        provider: resolved.provider.slug().to_owned(),
    })
}

pub(super) async fn metadata_field<Source, Transport>(
    target: &Target,
    key: &str,
    environment: &mut CapabilityEnvironment<'_, Source, Transport>,
) -> Result<MetadataValue, CapabilityError>
where
    Source: DynamicMetadataSource,
    Transport: ModelInfoClient,
{
    let request = MetadataRequest {
        model: &target.model,
        custom_llm_provider: Some(&target.provider),
        api_base: None,
        api_key: None,
    };
    let metadata = get_model_info_helper(
        &request,
        &environment.metadata,
        &mut *environment.keys,
        environment.dynamic,
        environment.hf,
    )
    .await?;
    let fields: BTreeMap<String, Box<RawValue>> = from_str(metadata.get())?;
    let Some(value) = fields.get(key).filter(|value| value.get() != "null") else {
        return Ok(MetadataValue::Absent);
    };
    Ok(MetadataValue::Value(from_str::<Json>(value.get())?))
}

// The unprefixed twin of a provider-prefixed entry, which the map may declare instead (#20885).
fn bare_field(
    model_cost: &IndexMap<String, Box<RawValue>>,
    keys: &mut ModelCostKeys,
    model: &str,
    key: &str,
) -> Option<Box<RawValue>> {
    let entry_key = keys.get_model_cost_key(model_cost, model)?;
    let entry = model_cost.get(&entry_key)?;
    let fields: BTreeMap<String, Box<RawValue>> = from_str(entry.get()).ok()?;
    let value = fields.get(key)?;
    // an unreadable entry answers nothing, which is what LiteLLM's dict check on the bare entry yields
    (value.get() != "null").then(|| value.clone())
}

pub(super) fn bare_is_true<Source, Transport>(
    model: &str,
    key: &str,
    environment: &mut CapabilityEnvironment<'_, Source, Transport>,
) -> bool {
    let metadata = &environment.metadata;
    let value = bare_field(metadata.model_cost, &mut *environment.keys, model, key);
    value.is_some_and(|raw| matches!(from_str::<bool>(raw.get()), Ok(true)))
}

pub(super) fn bare_is_false<Source, Transport>(
    model: &str,
    key: &str,
    environment: &mut CapabilityEnvironment<'_, Source, Transport>,
) -> bool {
    let metadata = &environment.metadata;
    let value = bare_field(metadata.model_cost, &mut *environment.keys, model, key);
    value.is_some_and(|raw| matches!(from_str::<bool>(raw.get()), Ok(false)))
}

pub(super) fn bare_text<Source, Transport>(
    model: &str,
    key: &str,
    environment: &mut CapabilityEnvironment<'_, Source, Transport>,
) -> Option<String> {
    let metadata = &environment.metadata;
    let value = bare_field(metadata.model_cost, &mut *environment.keys, model, key)?;
    from_str::<String>(value.get()).ok()
}

// get_provider_info answers True or None for a key; every other provider declares nothing for it.
pub(super) fn provider_flag<Source, Transport>(
    model: &str,
    provider: Option<&str>,
    key: &str,
    environment: &CapabilityEnvironment<'_, Source, Transport>,
) -> bool {
    let query = MetadataQuery {
        catalog: environment.provider_catalog,
        generation: environment.provider_generation,
    };
    let Some(info) = get_provider_info(model, provider, query) else {
        return false;
    };
    let declared = match key {
        "supports_function_calling" => info.supports_function_calling,
        "supports_prompt_caching" => info.supports_prompt_caching,
        "supports_reasoning" => info.supports_reasoning,
        "supports_vision" => info.supports_vision,
        "supports_pdf_input" => info.supports_pdf_input,
        "supports_web_search" => info.supports_web_search,
        _ => None,
    };
    declared == Some(true)
}

pub(super) fn log_failure(
    what: &str,
    model: &str,
    provider: Option<&str>,
    error: &CapabilityError,
) {
    tracing::debug!(
        "Model not found or error in {what}. You passed model={model}, custom_llm_provider={}. Error: {error}",
        provider.unwrap_or("None")
    );
}
