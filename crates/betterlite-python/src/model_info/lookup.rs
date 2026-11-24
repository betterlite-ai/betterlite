use super::convert::{ModelQuery, ProviderQuery};
use super::error::ModelInfoError;
use super::fetch::HttpFetch;
use super::state::{Runtime, State, normalizers};
use betterlite_models::catalog::ModelCatalog;
use betterlite_models::decisions::Json;
use betterlite_models::entry::ModelFields;
use betterlite_models::fireworks_catalog_metadata::{
    Catalog as FireworksCatalog, MODEL_COST_KEY_PREFIX,
};
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_models::lookup::capabilities::{CapabilityFlag, CapabilityView};
use betterlite_models::model_catalog_keys::ModelCostKeys;
use betterlite_models::model_metadata_resolution::dynamic::NoDynamicMetadata;
use betterlite_models::model_metadata_resolution::hf_config::{HF_CONFIG_BASE_URL, HfConfigClient};
use betterlite_models::model_metadata_resolution::{
    MetadataEnvironment, MetadataRequest, get_model_info_helper,
};
use betterlite_models::params::CallType;
use betterlite_models::params::capabilities::{
    CapabilityQuery, ModelProjection, OpenAiCapability, OpenAiConfigTag, ParameterCapabilities,
    supported_parameters,
};
use betterlite_models::provider_metadata_factory::adapters::ProviderSpecificModelInfo;
use betterlite_models::provider_metadata_factory::selection::{MetadataQuery, get_provider_info};
use betterlite_models::resolution::{
    ProviderIdentity, ResolutionContext, ResolutionRequest, resolve,
};
use betterlite_openai::openai_base_config::{CatalogCapabilities, ModelFamily, model_family};
use betterlite_openai::text_completion_execution::catalog::CompletionCatalog;
use indexmap::IndexMap;
use serde_json::value::RawValue;
use std::collections::{BTreeMap, BTreeSet};
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
// get_supported_openai_params infers the provider from model_cost too, so an entry Python just sent counts.
fn is_openai_model(
    resolution: &ResolutionContext<'_>,
    entries: &IndexMap<String, Box<RawValue>>,
    model: &str,
    provider: Option<&str>,
) -> bool {
    if provider.is_none()
        && let Some(listed) = entry_provider(entries, model)
    {
        return listed == "openai";
    }
    let request = ResolutionRequest {
        model,
        custom_llm_provider: provider,
        api_base: None,
        api_key: None,
    };
    resolve(&request, resolution)
        .is_ok_and(|resolved| resolved.provider == ProviderIdentity::Known(ProviderId::Openai))
}
fn entry_provider(entries: &IndexMap<String, Box<RawValue>>, model: &str) -> Option<String> {
    let entry = entries.get(model)?;
    let fields: BTreeMap<String, Box<RawValue>> = serde_json::from_str(entry.get()).ok()?;
    serde_json::from_str(fields.get("litellm_provider")?.get()).ok()
}
fn openai_parameters(
    state: &State,
    resolution: &ResolutionContext<'_>,
    model: &str,
    provider: Option<&str>,
) -> Option<Vec<String>> {
    if !is_openai_model(resolution, &state.entries, model, provider) {
        return None;
    }
    let completion = CompletionCatalog::new(&state.catalog);
    let catalog = CatalogCapabilities {
        chat_models: completion.chat_models,
        text_models: completion.text_models,
    };
    let capabilities = openai_capabilities(&state.catalog, model, &catalog);
    let query = CapabilityQuery {
        provider: ProviderId::Openai,
        call_type: CallType::ChatCompletion,
        config: config_tag(model, &catalog),
        model: ModelProjection {
            model,
            capabilities: &capabilities,
        },
        base_model: None,
    };
    let ParameterCapabilities::Listed(names) = supported_parameters(&query) else {
        return None;
    };
    Some(names.into_iter().map(str::to_owned).collect())
}
// LiteLLM picks the OpenAI config by family before it reads the parameter list, so gpt-5 and o-series differ from the base GPT list.
fn config_tag(model: &str, catalog: &CatalogCapabilities) -> OpenAiConfigTag {
    match model_family(model, catalog) {
        ModelFamily::OSeries => OpenAiConfigTag::OSeries,
        ModelFamily::GptReasoning => OpenAiConfigTag::Gpt5,
        ModelFamily::Audio | ModelFamily::Gpt => OpenAiConfigTag::Gpt,
        ModelFamily::Unknown => OpenAiConfigTag::UnknownModel,
    }
}
fn openai_capabilities(
    catalog: &ModelCatalog,
    model: &str,
    listed: &CatalogCapabilities,
) -> BTreeSet<OpenAiCapability> {
    let mut capabilities = BTreeSet::new();
    if listed.is_catalog_model(model) {
        capabilities.insert(OpenAiCapability::CatalogModel);
    }
    let found = catalog
        .lookup
        .get(model)
        .and_then(|canonical| catalog.entries.get(canonical.as_ref()));
    let Some(entry) = found else {
        return capabilities;
    };
    let fields = &entry.fields.capabilities;
    let view = CapabilityView::from_fields(fields);
    for (flag, capability) in [
        (
            CapabilityFlag::FunctionCalling,
            OpenAiCapability::FunctionCalling,
        ),
        (CapabilityFlag::ToolChoice, OpenAiCapability::ToolChoice),
        (
            CapabilityFlag::ResponseSchema,
            OpenAiCapability::ResponseSchema,
        ),
    ] {
        if view.get(flag) == Some(true) {
            capabilities.insert(capability);
        }
    }
    if fields.tooling.supports_parallel_calls == Some(true) {
        capabilities.insert(OpenAiCapability::ParallelToolCalls);
    }
    if fields.reasoning.supports_none_reasoning_effort == Some(true) {
        capabilities.insert(OpenAiCapability::NoneReasoningEffort);
    }
    capabilities
}
mod tests;
