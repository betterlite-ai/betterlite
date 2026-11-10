use indexmap::IndexMap;
use secrecy::SecretString;
use serde_json::value::RawValue;

use crate::catalog_generalizations::CatalogGeneralizations;
use crate::identity::declared::authenticating_provider;
use crate::lemonade_model_metadata::ModelInfoClient;
use crate::model_catalog_keys::{ModelCostKeys, PotentialModelNames};
use crate::resolution::ResolutionContext;

pub mod dynamic;
pub mod error;
pub mod hf_config;
mod lookup;
pub mod names;

use dynamic::DynamicMetadataSource;
use error::{ModelMetadataError, StaticFailure};
use hf_config::{HfConfigClient, huggingface_model_info};
use lookup::{Lookup, exact_entry, generalization, project_entry};
use names::{
    NameNormalizers, PotentialNamesInput, VertexModelSets, apply_aliases, potential_model_names,
};

pub struct MetadataRequest<'request> {
    pub model: &'request str,
    pub custom_llm_provider: Option<&'request str>,
    pub api_base: Option<&'request str>,
    pub api_key: Option<&'request SecretString>,
}

pub struct MetadataEnvironment<'environment> {
    pub model_cost: &'environment IndexMap<String, Box<RawValue>>,
    pub vertex: &'environment VertexModelSets,
    pub generalizations: &'environment CatalogGeneralizations,
    pub resolution: &'environment ResolutionContext<'environment>,
    pub normalizers: NameNormalizers,
}

pub async fn get_model_info_helper<Source, Transport>(
    request: &MetadataRequest<'_>,
    environment: &MetadataEnvironment<'_>,
    keys: &mut ModelCostKeys,
    dynamic: &Source,
    hf: &HfConfigClient<Transport>,
) -> Result<Box<RawValue>, ModelMetadataError>
where
    Source: DynamicMetadataSource,
    Transport: ModelInfoClient,
{
    let mut model = request.model.to_owned();
    let mut provider = request.custom_llm_provider.map(str::to_owned);
    apply_aliases(&mut model, &mut provider, environment.vertex);
    let names = potential_model_names(&PotentialNamesInput {
        model: &model,
        custom_llm_provider: declared_provider(&model, provider.as_deref()),
        resolution: environment.resolution,
        normalizers: environment.normalizers,
    });
    provider = names.custom_llm_provider.clone();
    if let Some(value) = dynamic::dynamic_model_info(
        dynamic,
        &model,
        provider.as_deref(),
        request.api_base,
        request.api_key,
    )
    .await
    {
        return Ok(value);
    }
    if provider.as_deref() == Some("huggingface") {
        let max_tokens = hf.max_position_embeddings(&model).await;
        return huggingface_model_info(&model, max_tokens)
            .map_err(|source| failed(model, provider, StaticFailure::from(source)));
    }
    match resolve_static(&model, provider.as_deref(), &names, environment, keys) {
        Ok(value) => Ok(value),
        Err(StaticError::NotMapped) => Err(ModelMetadataError::NotMapped { model, provider }),
        Err(StaticError::Failed(source)) => Err(failed(model, provider, source)),
    }
}

fn declared_provider<'provider>(
    model: &'provider str,
    custom_llm_provider: Option<&'provider str>,
) -> Option<&'provider str> {
    custom_llm_provider
        .or_else(|| authenticating_provider(Some(model), None).map(<&'static str>::from))
}

enum StaticError {
    NotMapped,
    Failed(StaticFailure),
}

fn failed(model: String, provider: Option<String>, source: StaticFailure) -> ModelMetadataError {
    tracing::debug!("Error getting model info: {source}");
    ModelMetadataError::Failed {
        model,
        provider,
        source,
    }
}

fn resolve_static(
    model: &str,
    provider: Option<&str>,
    names: &PotentialModelNames,
    environment: &MetadataEnvironment<'_>,
    keys: &mut ModelCostKeys,
) -> Result<Box<RawValue>, StaticError> {
    let lookup = Lookup {
        model,
        provider,
        names,
        model_cost: environment.model_cost,
        generalizations: environment.generalizations,
    };
    let exact = exact_entry(&lookup, keys);
    if let Some(matched) = exact.map_err(StaticError::Failed)? {
        return project_entry(&matched, model, provider).map_err(StaticError::Failed);
    }
    let generalized = generalization(&lookup, keys);
    if let Some(matched) = generalized.map_err(StaticError::Failed)? {
        return project_entry(&matched, model, provider).map_err(StaticError::Failed);
    }
    Err(StaticError::NotMapped)
}

mod tests;
