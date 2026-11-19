use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::seams::{ModelMetadata, ProviderLookup, ReasoningSeams};
use super::values::{carries_value, text};
use super::{
    CUSTOM_LLM_PROVIDER, DETAILED, EFFORT, GENERATE_SUMMARY, MODEL, REASONING_EFFORT,
    RESPONSES_PREFIX, SUMMARY,
};
use crate::api::messages_thinking::bridge::local_model_name;
use crate::api::messages_thinking::effort::{degrade_effort, normalize_reasoning_effort};

// the enabled thinking map's summary reaches the reasoning dict the routed call is given
pub(super) fn convert_effort(
    kwargs: &mut super::CompletionKwargs,
    thinking: &IndexMap<String, Json>,
    auto_summary: bool,
) {
    let summary = thinking.get(SUMMARY).cloned();
    match kwargs.fields.get(REASONING_EFFORT).cloned() {
        Some(Json::Text(effort)) if !effort.is_empty() => {
            let effort = effort_object(&effort, summary.as_ref(), auto_summary);
            kwargs
                .fields
                .insert(REASONING_EFFORT.to_owned(), Json::Object(effort));
        }
        Some(Json::Object(reasoning)) => {
            if let Some(updated) = summary_object(reasoning, summary.as_ref(), auto_summary) {
                kwargs
                    .fields
                    .insert(REASONING_EFFORT.to_owned(), Json::Object(updated));
            }
        }
        None
        | Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {}
    }
}

// the string branch always writes a fresh dict, with the thinking summary ahead of the auto one
fn effort_object(
    effort: &str,
    summary: Option<&Json>,
    auto_summary: bool,
) -> IndexMap<String, Json> {
    let mut reasoning = IndexMap::new();
    reasoning.insert(EFFORT.to_owned(), Json::Text(effort.to_owned()));
    if let Some(summary) = summary.filter(|value| carries_value(value)) {
        reasoning.insert(SUMMARY.to_owned(), summary.clone());
    } else if auto_summary {
        reasoning.insert(SUMMARY.to_owned(), Json::Text(DETAILED.to_owned()));
    }
    reasoning
}

// the dict branch only fills a gap: a present summary or generate_summary key, null included, is the caller's answer
fn summary_object(
    mut reasoning: IndexMap<String, Json>,
    summary: Option<&Json>,
    auto_summary: bool,
) -> Option<IndexMap<String, Json>> {
    if reasoning.contains_key(SUMMARY) || reasoning.contains_key(GENERATE_SUMMARY) {
        return None;
    }
    let effective = summary
        .filter(|value| carries_value(value))
        .cloned()
        .or_else(|| auto_summary.then(|| Json::Text(DETAILED.to_owned())))?;
    reasoning.insert(SUMMARY.to_owned(), effective);
    Some(reasoning)
}

// an absent or null provider is inferred; any other value is taken as given, even when it is not openai
pub(super) fn resolve_provider(
    kwargs: &super::CompletionKwargs,
    seams: &ReasoningSeams<'_>,
) -> Option<String> {
    match kwargs.fields.get(CUSTOM_LLM_PROVIDER) {
        Some(Json::Text(provider)) => Some(provider.clone()),
        None | Some(Json::Null) => seams
            .provider
            .resolve(&ProviderLookup {
                model: text(kwargs.fields.get(MODEL)),
                custom_llm_provider: None,
                api_base: None,
                api_key: None,
            })
            .map(|target| target.provider),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => None,
    }
}

// a tier outside the degradation chain comes back unchanged before any lookup (utils.py L84-L86)
pub(super) fn normalized_effort(
    effort: &str,
    model: Option<&str>,
    provider: Option<&str>,
    metadata: &dyn ModelMetadata,
) -> String {
    if degrade_effort(effort, None) == effort {
        return effort.to_owned();
    }
    let info = metadata.model_info(model, provider);
    normalize_reasoning_effort(effort, info.as_ref(), metadata.model_cost()).to_owned()
}

// the lookup reads the model and provider values themselves, and the route step's own prefix is not among them yet
pub(super) fn rewrite_model(kwargs: &mut super::CompletionKwargs, provider: &str) {
    let Some(Json::Text(model)) = kwargs.fields.get(MODEL) else {
        return;
    };
    if model.is_empty() || model.contains(RESPONSES_PREFIX) {
        return;
    }
    let local = local_model_name(model, Some(provider));
    let rewritten = format!("{provider}/{RESPONSES_PREFIX}{local}");
    kwargs
        .fields
        .insert(MODEL.to_owned(), Json::Text(rewritten));
}

// the truthy thinking value, a reasoning dict carrying a summary key, a string effort and a non-empty model
pub(super) fn plain_target<'kwargs>(
    kwargs: &'kwargs super::CompletionKwargs,
    thinking: Option<&Json>,
) -> Option<(&'kwargs IndexMap<String, Json>, &'kwargs str, &'kwargs str)> {
    if !thinking.is_some_and(carries_value) {
        return None;
    }
    let Some(Json::Object(reasoning)) = kwargs.fields.get(REASONING_EFFORT) else {
        return None;
    };
    if !reasoning.contains_key(SUMMARY) {
        return None;
    }
    let Json::Text(effort) = reasoning.get(EFFORT)? else {
        return None;
    };
    let Json::Text(model) = kwargs.fields.get(MODEL)? else {
        return None;
    };
    if model.is_empty() {
        return None;
    }
    Some((reasoning, effort.as_str(), model.as_str()))
}
