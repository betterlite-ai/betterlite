use betterlite_models::entry::ModelFields;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_models::lookup::capabilities::{CapabilityFlag, CapabilityView};
use betterlite_models::params::CallType;
use betterlite_models::params::capabilities::{
    CapabilityQuery, ModelProjection, OpenAiCapability, OpenAiConfigTag, ParameterCapabilities,
    supported_parameters,
};
use betterlite_models::resolution::{
    ProviderIdentity, ResolutionContext, ResolutionRequest, resolve,
};
use betterlite_openai::openai_base_config::{CatalogCapabilities, ModelFamily, model_family};
use betterlite_openai::text_completion_execution::catalog::CompletionCatalog;
use indexmap::IndexMap;
use serde_json::value::RawValue;
use std::collections::{BTreeMap, BTreeSet};

use crate::model_info::state::State;

pub(super) fn openai_parameters(
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
    // LiteLLM's supports_* helpers resolve the name through get_llm_provider, so a prefixed id reads the bare model's fields.
    let resolved = resolve(
        &ResolutionRequest {
            model,
            custom_llm_provider: provider,
            api_base: None,
            api_key: None,
        },
        resolution,
    )
    .map_or_else(|_error| model.to_owned(), |found| found.model);
    let capabilities = openai_capabilities(&state.entries, model, &resolved, &catalog);
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
// get_supported_openai_params infers the provider from model_cost too, so an entry Python just sent counts.
pub(super) fn is_openai_model(
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
    entries: &IndexMap<String, Box<RawValue>>,
    given: &str,
    resolved: &str,
    listed: &CatalogCapabilities,
) -> BTreeSet<OpenAiCapability> {
    let mut capabilities = BTreeSet::new();
    if listed.is_catalog_model(given) {
        capabilities.insert(OpenAiCapability::CatalogModel);
    }
    // LiteLLM's supports_* helpers read the model_cost entry, so a synced capability change counts.
    let Some(fields) = entries
        .get(resolved)
        .and_then(|raw| ModelFields::parse(raw).ok())
        .map(|parsed| parsed.0)
    else {
        return capabilities;
    };
    let view = CapabilityView::from_fields(&fields.capabilities);
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
    if fields.capabilities.tooling.supports_parallel_calls == Some(true) {
        capabilities.insert(OpenAiCapability::ParallelToolCalls);
    }
    if fields.capabilities.reasoning.supports_none_reasoning_effort == Some(true) {
        capabilities.insert(OpenAiCapability::NoneReasoningEffort);
    }
    capabilities
}
