use std::convert::Infallible;
use std::sync::{Arc, Mutex};

use crate::lemonade_model_metadata::ModelInfoClient;
use crate::model_catalog_keys::ModelCostKeys;
use crate::model_metadata_resolution::dynamic::DynamicMetadataSource;
use crate::model_metadata_resolution::hf_config::HfConfigClient;
use crate::model_metadata_resolution::{
    MetadataEnvironment, MetadataRequest, get_model_info_helper,
};
use crate::provider_metadata_factory::selection::{MetadataQuery, get_provider_info};

pub mod cache;
pub mod error;
pub mod parameters;
pub mod record;

use cache::{CacheKey, MetadataCaches, ModelInfoCache};
use error::FullMetadataError;
use parameters::SupportedParameterSource;
use record::{ModelInfo, ModelInfoHandle};

mod tests;

// _cached_get_model_info_helper takes no api_key, so its cache never holds an authenticated answer.
pub struct HelperQuery<'query> {
    pub model: &'query str,
    pub custom_llm_provider: Option<&'query str>,
    pub api_base: Option<&'query str>,
}

pub struct HelperSources<'source, 'environment, Source, Transport> {
    pub environment: &'source MetadataEnvironment<'environment>,
    pub dynamic: &'source Source,
    pub hf: &'source HfConfigClient<Transport>,
}

pub struct FullMetadataSources<'source, 'environment, Source, Transport, Parameters> {
    pub helper: HelperSources<'source, 'environment, Source, Transport>,
    pub provider_query: MetadataQuery<'source>,
    pub parameters: &'source Parameters,
}

pub async fn get_model_info<Source, Transport, Parameters>(
    request: &MetadataRequest<'_>,
    sources: &FullMetadataSources<'_, '_, Source, Transport, Parameters>,
    keys: &mut ModelCostKeys,
    caches: &MetadataCaches,
) -> Result<ModelInfoHandle, FullMetadataError<Parameters::Error>>
where
    Source: DynamicMetadataSource,
    Transport: ModelInfoClient,
    Parameters: SupportedParameterSource,
{
    let key = CacheKey::for_request(request);
    let existing = if request.api_key.is_some() {
        None
    } else {
        caches.full.get(&key)
    };
    if let Some(handle) = existing {
        return Ok(handle);
    }
    // the parameter query runs first, so its failure skips the helper and its metadata HTTP
    let supported = sources
        .parameters
        .supported_openai_params(request.model, request.custom_llm_provider)
        .map_err(|source| FullMetadataError::Parameters { source })?;
    let metadata = get_model_info_helper(
        request,
        sources.helper.environment,
        keys,
        sources.helper.dynamic,
        sources.helper.hf,
    )
    .await?;
    let overlay = get_provider_info(
        request.model,
        request.custom_llm_provider,
        sources.provider_query,
    );
    let handle = Arc::new(Mutex::new(record::build(
        &metadata,
        overlay.as_ref(),
        supported,
    )?));
    if request.api_key.is_none() {
        caches.full.put(key, Arc::clone(&handle));
    }
    Ok(handle)
}

pub async fn get_model_info_helper_cached<Source, Transport>(
    query: &HelperQuery<'_>,
    sources: &HelperSources<'_, '_, Source, Transport>,
    keys: &mut ModelCostKeys,
    cache: &ModelInfoCache,
) -> Result<ModelInfoHandle, FullMetadataError<Infallible>>
where
    Source: DynamicMetadataSource,
    Transport: ModelInfoClient,
{
    let request = MetadataRequest {
        model: query.model,
        custom_llm_provider: query.custom_llm_provider,
        api_base: query.api_base,
        api_key: None,
    };
    let key = CacheKey::for_request(&request);
    if let Some(handle) = cache.get(&key) {
        return Ok(handle);
    }
    let metadata = get_model_info_helper(
        &request,
        sources.environment,
        keys,
        sources.dynamic,
        sources.hf,
    )
    .await?;
    let handle = Arc::new(Mutex::new(ModelInfo::parse(&metadata)?));
    cache.put(key, Arc::clone(&handle));
    Ok(handle)
}
