use betterlite_models::entry::ModelFields;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_models::lookup::capabilities::{CapabilityFlag, CapabilityView};
use betterlite_models::model_catalog_keys::{
    EntryProvider, ModelCostKeys, PotentialModelNames, check_provider_match,
};
use betterlite_models::model_metadata_resolution::names::{
    PotentialNamesInput, potential_model_names,
};
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

use crate::model_info::state::{State, normalizers};

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
    let capabilities = openai_capabilities(state, resolution, model, provider, &catalog);
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
// LiteLLM's supports_* helpers read the model_cost entry under provider/model before the bare name.
fn openai_capabilities(
    state: &State,
    resolution: &ResolutionContext<'_>,
    model: &str,
    provider: Option<&str>,
    listed: &CatalogCapabilities,
) -> BTreeSet<OpenAiCapability> {
    let mut capabilities = BTreeSet::new();
    if listed.is_catalog_model(model) {
        capabilities.insert(OpenAiCapability::CatalogModel);
    }
    // _supports_factory runs after get_llm_provider, so the lookups carry the resolved OpenAI provider.
    let effective = provider.unwrap_or("openai");
    let names = potential_model_names(&PotentialNamesInput {
        model,
        custom_llm_provider: Some(effective),
        resolution,
        normalizers: normalizers(),
    });
    let (matched, bare) = capability_entries(
        &state.entries,
        model,
        &names,
        names.custom_llm_provider.as_deref(),
    );
    capabilities.extend(entry_capabilities(matched.as_ref(), bare.as_ref()));
    capabilities
}
fn entry_capabilities(
    matched: Option<&ModelFields>,
    bare: Option<&ModelFields>,
) -> BTreeSet<OpenAiCapability> {
    let mut capabilities = BTreeSet::new();
    let matched_view = matched.map(|fields| CapabilityView::from_fields(&fields.capabilities));
    let bare_view = bare.map(|fields| CapabilityView::from_fields(&fields.capabilities));
    // _supports_factory falls back to the bare entry only for a flag the matched entry leaves out.
    let flags = matched_view
        .unwrap_or_default()
        .or(bare_view.unwrap_or_default());
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
        if flags.get(flag) == Some(true) {
            capabilities.insert(capability);
        }
    }
    let matched_parallel =
        matched.and_then(|fields| fields.capabilities.tooling.supports_parallel_calls);
    let bare_parallel = bare.and_then(|fields| fields.capabilities.tooling.supports_parallel_calls);
    if matched_parallel.or(bare_parallel) == Some(true) {
        capabilities.insert(OpenAiCapability::ParallelToolCalls);
    }
    let matched_none_effort =
        matched.and_then(|fields| fields.capabilities.reasoning.supports_none_reasoning_effort);
    let bare_none_effort =
        bare.and_then(|fields| fields.capabilities.reasoning.supports_none_reasoning_effort);
    if matched_none_effort.or(bare_none_effort) == Some(true) {
        capabilities.insert(OpenAiCapability::NoneReasoningEffort);
    }
    capabilities
}
// The exact-entry ladder _get_model_info_helper walks, and the bare twin #20885 falls back to for a missing flag.
fn capability_entries(
    entries: &IndexMap<String, Box<RawValue>>,
    model: &str,
    names: &PotentialModelNames,
    provider: Option<&str>,
) -> (Option<ModelFields>, Option<ModelFields>) {
    let mut keys = ModelCostKeys::default();
    let matched = candidates(model, names).into_iter().find_map(|candidate| {
        let key = keys.get_model_cost_key(entries, candidate)?;
        let raw = entries.get(&key)?;
        if !provider_matches(raw, provider) {
            return None;
        }
        ModelFields::parse(raw).ok().map(|parsed| parsed.0)
    });
    let bare = keys
        .get_model_cost_key(entries, &names.split_model)
        .and_then(|key| entries.get(&key))
        .and_then(|raw| ModelFields::parse(raw).ok())
        .map(|parsed| parsed.0);
    (matched, bare)
}
fn candidates<'names>(model: &'names str, names: &'names PotentialModelNames) -> [&'names str; 6] {
    [
        &names.combined_model_name,
        model,
        &names.split_model,
        &names.combined_stripped_model_name,
        &names.stripped_model_name,
        &names.provider_prefixed_model_name,
    ]
}
// LiteLLM's provider check treats a missing or null litellm_provider as a wildcard.
fn provider_matches(raw: &RawValue, provider: Option<&str>) -> bool {
    let Ok(fields) = serde_json::from_str::<BTreeMap<String, Box<RawValue>>>(raw.get()) else {
        return false;
    };
    let entry_provider =
        fields
            .get("litellm_provider")
            .map_or(EntryProvider::Unconstrained, |value| {
                serde_json::from_str::<Option<&str>>(value.get())
                    .map_or(EntryProvider::NotText, |name| {
                        name.map_or(EntryProvider::Unconstrained, EntryProvider::Text)
                    })
            });
    check_provider_match(entry_provider, provider).unwrap_or(false)
}
