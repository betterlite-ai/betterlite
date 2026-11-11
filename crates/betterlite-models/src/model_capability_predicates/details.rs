use std::collections::BTreeMap;

use serde_json::from_str;
use serde_json::value::RawValue;

use crate::decisions::Json;
use crate::identity::declared::authenticating_provider;
use crate::lemonade_model_metadata::ModelInfoClient;
use crate::model_metadata_resolution::MetadataRequest;
use crate::model_metadata_resolution::dynamic::DynamicMetadataSource;
use crate::model_metadata_resolution::error::ModelMetadataError;
use crate::model_metadata_resolution::get_model_info_helper;
use crate::provider_metadata_factory::selection::{MetadataQuery, get_provider_info};
use crate::resolution::{ResolutionError, ResolutionRequest, resolve};

use super::raw::{bare_declaration, raw_metadata};
use super::{CapabilityEnvironment, CapabilityRequest};

// Python's dict.get: absent and null read the same, and any other value is compared by identity or read as text.
#[derive(Debug, PartialEq, Eq)]
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

// What LiteLLM's provider-info fallback reads after a failure: the declared pair, or the resolved one.
pub(super) struct Failure {
    pub(super) model: String,
    pub(super) provider: Option<String>,
}

// get_llm_provider returns the declared provider pair itself, skipping the OAuth flow its resolution runs.
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

// get_llm_provider, which the string readers and streaming call without the declared fast path.
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

// Which provider pair the declarations below resolve before reading the metadata.
#[derive(Clone, Copy)]
pub(super) enum Strategy {
    DeclaredFirst,
    Resolved,
}

// The debug line's middle phrase; LiteLLM names each reader's failure differently.
#[derive(Clone, Copy)]
pub(super) enum Phrase {
    Support,
    Reading,
    DisabledState,
}

impl Phrase {
    fn of(self, key: &str) -> String {
        match self {
            Self::Support => format!("checking {key} support"),
            Self::Reading => format!("reading {key}"),
            Self::DisabledState => format!("checking {key} disabled state"),
        }
    }
}

// The target and its metadata value, or the pair a failed lookup hands to the provider-info fallback.
pub(super) async fn lookup<Source, Transport>(
    request: &CapabilityRequest<'_>,
    key: &str,
    strategy: Strategy,
    phrase: Phrase,
    environment: &mut CapabilityEnvironment<'_, Source, Transport>,
) -> Result<(Target, MetadataValue), Failure>
where
    Source: DynamicMetadataSource,
    Transport: ModelInfoClient,
{
    let what = phrase.of(key);
    let target = resolvable(request, strategy, &what, environment)?;
    match metadata_field(&target, key, environment).await {
        Ok(value) => Ok((target, value)),
        Err(error) => {
            // declared_value_factory never rebinds its pair, so its failure line names the caller's model
            let (model, provider) = match phrase {
                Phrase::Reading => (request.model, request.custom_llm_provider),
                Phrase::Support | Phrase::DisabledState => {
                    (target.model.as_str(), Some(target.provider.as_str()))
                }
            };
            log_failure(&what, model, provider, &error);
            Err(Failure {
                model: target.model,
                provider: Some(target.provider),
            })
        }
    }
}

// A resolution failure logs its own phrase and hands the declared pair to the provider-info fallback.
fn resolvable<Source, Transport>(
    request: &CapabilityRequest<'_>,
    strategy: Strategy,
    what: &str,
    environment: &CapabilityEnvironment<'_, Source, Transport>,
) -> Result<Target, Failure> {
    let resolved = match strategy {
        Strategy::DeclaredFirst => {
            declared_target(request).map_or_else(|| resolved_target(request, environment), Ok)
        }
        Strategy::Resolved => resolved_target(request, environment),
    };
    resolved.map_err(|error| {
        log_failure(
            what,
            request.model,
            request.custom_llm_provider,
            &CapabilityError::from(error),
        );
        Failure {
            model: request.model.to_owned(),
            provider: request.custom_llm_provider.map(str::to_owned),
        }
    })
}

// A metadata None answer reads the bare entry for an exact True, then the provider info for one.
pub(super) fn fallback_answer<Source, Transport>(
    target: &Target,
    key: &str,
    environment: &mut CapabilityEnvironment<'_, Source, Transport>,
) -> bool {
    matches!(
        bare_declaration(&target.model, key, environment),
        Some(Json::Bool(true))
    ) || provider_flag(&target.model, Some(&target.provider), key, environment)
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
    let metadata = match get_model_info_helper(
        &request,
        &environment.metadata,
        &mut *environment.keys,
        environment.dynamic,
        environment.hf,
    )
    .await
    {
        Ok(value) => value,
        // B2219's projection types capability flags as bool, so a string value fails its parse even though LiteLLM's dict read returns it
        Err(error) => match raw_metadata(target, key, environment) {
            Some(value) => return Ok(value),
            None => return Err(CapabilityError::Metadata(error)),
        },
    };
    let fields: BTreeMap<String, Box<RawValue>> = from_str(metadata.get())?;
    let Some(value) = fields.get(key).filter(|value| value.get() != "null") else {
        return Ok(MetadataValue::Absent);
    };
    Ok(MetadataValue::Value(from_str::<Json>(value.get())?))
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
