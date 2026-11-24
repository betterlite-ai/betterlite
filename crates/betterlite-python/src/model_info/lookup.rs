use super::convert::{ModelQuery, ProviderQuery};
use super::error::ModelInfoError;
use super::fetch::HttpFetch;
use super::state::{Runtime, State, normalizers};
use betterlite_models::decisions::Json;
use betterlite_models::entry::ModelFields;
use betterlite_models::fireworks_catalog_metadata::{
    Catalog as FireworksCatalog, MODEL_COST_KEY_PREFIX,
};
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_models::model_catalog_keys::ModelCostKeys;
use betterlite_models::model_metadata_resolution::dynamic::NoDynamicMetadata;
use betterlite_models::model_metadata_resolution::hf_config::{HF_CONFIG_BASE_URL, HfConfigClient};
use betterlite_models::model_metadata_resolution::{
    MetadataEnvironment, MetadataRequest, get_model_info_helper,
};
use betterlite_models::provider_metadata_factory::adapters::ProviderSpecificModelInfo;
use betterlite_models::provider_metadata_factory::selection::{MetadataQuery, get_provider_info};
use betterlite_models::resolution::{ProviderIdentity, ResolutionRequest, resolve};
use indexmap::IndexMap;
use serde_json::value::RawValue;
use std::collections::BTreeMap;

use self::openai_params::openai_parameters;
pub(crate) async fn model_info(
    runtime: &Runtime,
    query: &ModelQuery,
) -> Result<String, ModelInfoError> {
    let state = runtime.state();
    let resolution = runtime.resolution(&state);
    let parameters = openai_parameters(
        &state,
        &resolution,
        &query.model,
        query.custom_llm_provider.as_deref(),
    );
    let request = MetadataRequest {
        model: &query.model,
        custom_llm_provider: query.custom_llm_provider.as_deref(),
        api_base: query.api_base.as_deref(),
        api_key: query.api_key.as_ref(),
    };
    let environment = MetadataEnvironment {
        model_cost: &state.entries,
        vertex: &state.vertex,
        generalizations: &state.generalizations,
        resolution: &resolution,
        normalizers: normalizers(),
    };
    let metadata = metadata_text(&request, &environment, &runtime.fetch).await?;
    let mut fields: IndexMap<String, Json> =
        serde_json::from_str(metadata.get()).map_err(ModelInfoError::Request)?;
    if let Some(overlay) =
        provider_overlay(&state, &query.model, query.custom_llm_provider.as_deref())
    {
        let text = serde_json::to_string(&overlay).map_err(ModelInfoError::Result)?;
        let overlay_fields: IndexMap<String, Json> =
            serde_json::from_str(&text).map_err(ModelInfoError::Request)?;
        fields.extend(overlay_fields);
    }
    let supported = parameters.map_or(Json::Null, |names| {
        Json::List(names.into_iter().map(Json::Text).collect())
    });
    drop(fields.insert("supported_openai_params".to_owned(), supported));
    serde_json::to_string(&fields).map_err(ModelInfoError::Result)
}
async fn metadata_text(
    request: &MetadataRequest<'_>,
    environment: &MetadataEnvironment<'_>,
    fetch: &HttpFetch,
) -> Result<Box<RawValue>, ModelInfoError> {
    let hf = HfConfigClient {
        client: fetch.clone(),
        base_url: HF_CONFIG_BASE_URL.to_owned(),
    };
    let mut keys = ModelCostKeys::default();
    get_model_info_helper(request, environment, &mut keys, &NoDynamicMetadata, &hf)
        .await
        .map_err(ModelInfoError::from)
}
// get_max_tokens: an exact model_cost key first, then the resolved name, with the unmapped error LiteLLM raises.
pub(crate) async fn max_tokens(
    runtime: &Runtime,
    model: &str,
) -> Result<Option<i64>, ModelInfoError> {
    let state = runtime.state();
    if let ExactMaxTokens::Value(found) = exact_max_tokens(&state.entries, model)? {
        return Ok(found);
    }
    let resolution = runtime.resolution(&state);
    let resolved = resolve(
        &ResolutionRequest {
            model,
            custom_llm_provider: None,
            api_base: None,
            api_key: None,
        },
        &resolution,
    )
    .map_err(|_error| ModelInfoError::MaxTokens {
        model: model.to_owned(),
    })?;
    if resolved.provider == ProviderIdentity::Known(ProviderId::Huggingface) {
        let hf = HfConfigClient {
            client: runtime.fetch.clone(),
            base_url: HF_CONFIG_BASE_URL.to_owned(),
        };
        return Ok(hf.max_position_embeddings(&resolved.model).await);
    }
    match exact_max_tokens(&state.entries, &resolved.model)? {
        ExactMaxTokens::Value(found) => Ok(found),
        ExactMaxTokens::Absent if state.entries.contains_key(&resolved.model) => Ok(None),
        ExactMaxTokens::Absent => Err(ModelInfoError::MaxTokens {
            model: resolved.model,
        }),
    }
}
enum ExactMaxTokens {
    Absent,
    Value(Option<i64>),
}
// Absent means no key is present, so the caller carries on to the resolved name, as LiteLLM's first check falls through.
fn exact_max_tokens(
    entries: &IndexMap<String, Box<RawValue>>,
    model: &str,
) -> Result<ExactMaxTokens, ModelInfoError> {
    let Some(entry) = entries.get(model) else {
        return Ok(ExactMaxTokens::Absent);
    };
    let fields: BTreeMap<String, Box<RawValue>> =
        serde_json::from_str(entry.get()).map_err(ModelInfoError::Request)?;
    for name in ["max_output_tokens", "max_tokens"] {
        if let Some(value) = fields.get(name) {
            return serde_json::from_str(value.get())
                .map(ExactMaxTokens::Value)
                .map_err(ModelInfoError::Request);
        }
    }
    Ok(ExactMaxTokens::Absent)
}
pub(crate) fn provider_info(
    runtime: &Runtime,
    query: &ProviderQuery,
) -> Result<Option<String>, ModelInfoError> {
    let state = runtime.state();
    let Some(info) = provider_overlay(&state, &query.model, query.custom_llm_provider.as_deref())
    else {
        return Ok(None);
    };
    serde_json::to_string(&info)
        .map(Some)
        .map_err(ModelInfoError::Result)
}
fn provider_overlay(
    state: &State,
    model: &str,
    provider: Option<&str>,
) -> Option<ProviderSpecificModelInfo> {
    let catalog = if provider == Some("fireworks_ai") {
        fireworks_catalog(&state.entries, model)
    } else {
        FireworksCatalog::new()
    };
    get_provider_info(
        model,
        provider,
        MetadataQuery {
            catalog: &catalog,
            generation: state.generation,
        },
    )
}
#[cfg_attr(test, mutants::skip)]
fn fireworks_catalog(entries: &IndexMap<String, Box<RawValue>>, model: &str) -> FireworksCatalog {
    entries
        .iter()
        .filter(|(key, _)| key.starts_with(MODEL_COST_KEY_PREFIX) || key.as_str() == model)
        .filter_map(|(key, raw)| {
            let fields: ModelFields = serde_json::from_str(raw.get()).ok()?;
            Some((key.clone(), fields))
        })
        .collect()
}
mod openai_params;
mod tests;
