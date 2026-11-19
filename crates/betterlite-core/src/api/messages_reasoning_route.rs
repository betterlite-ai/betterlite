pub mod seams;

mod route_steps;
mod tests;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use self::route_steps::{
    convert_effort, normalized_effort, plain_target, resolve_provider, rewrite_model,
};
use self::seams::{
    BridgeRequest, ModelMetadata, ProviderLookup, ReasoningRouteError, ReasoningSeams,
};
use crate::api::chat_response_stream_tools::fields::object;
use crate::api::messages_thinking::summary::auto_summary_enabled;
use crate::meta_muse_realtime::events::text_field;

const API_BASE: &str = "api_base";
const API_KEY: &str = "api_key";
const CUSTOM_LLM_PROVIDER: &str = "custom_llm_provider";
const DETAILED: &str = "detailed";
const EFFORT: &str = "effort";
const ENABLED: &str = "enabled";
const GENERATE_SUMMARY: &str = "generate_summary";
const LITELLM_PROXY: &str = "litellm_proxy";
const MODEL: &str = "model";
const OPENAI: &str = "openai";
const REASONING_EFFORT: &str = "reasoning_effort";
const RESPONSES_MODE: &str = "responses";
const RESPONSES_PREFIX: &str = "responses/";
const SUMMARY: &str = "summary";
const SUPPORTS_REASONING: &str = "supports_reasoning";
const THINKING_TYPE: &str = "type";
const TOOLS: &str = "tools";
const WEB_SEARCH_OPTIONS: &str = "web_search_options";

// _CompletionKwargs: the caller's map, with every field this path reads kept as the JSON value it arrived as
pub struct CompletionKwargs {
    pub fields: IndexMap<String, Json>,
}

// is_reasoning_auto_summary_enabled's two sources: the setting and the environment string
#[derive(Clone, Copy)]
pub struct AutoSummary<'settings> {
    pub global: bool,
    pub environment: Option<&'settings str>,
}

// _normalize_reasoning_effort
pub fn normalize_effort(kwargs: &mut CompletionKwargs, metadata: &dyn ModelMetadata) {
    let Some(reasoning) = kwargs.fields.get(REASONING_EFFORT).cloned() else {
        return;
    };
    // an absent model reads as "", and null stays the None get_model_info raises on
    let model = kwargs
        .fields
        .get(MODEL)
        .map_or(Some(""), |inner| text_field(Some(inner)));
    let provider = text_field(kwargs.fields.get(CUSTOM_LLM_PROVIDER));
    match reasoning {
        Json::Text(effort) => {
            let normalized = normalized_effort(&effort, model, provider, metadata);
            if normalized != effort {
                kwargs
                    .fields
                    .insert(REASONING_EFFORT.to_owned(), Json::Text(normalized));
            }
        }
        Json::Object(mut object) => {
            let Some(Json::Text(effort)) = object.get(EFFORT).cloned() else {
                return;
            };
            let normalized = normalized_effort(&effort, model, provider, metadata);
            if normalized != effort {
                object.insert(EFFORT.to_owned(), Json::Text(normalized));
                kwargs
                    .fields
                    .insert(REASONING_EFFORT.to_owned(), Json::Object(object));
            }
        }
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) => {}
    }
}

// _route_openai_thinking_to_responses_api_if_needed
pub fn route_thinking(
    kwargs: &mut CompletionKwargs,
    thinking: Option<&Json>,
    settings: AutoSummary<'_>,
    seams: &ReasoningSeams<'_>,
) {
    let Some(provider) = resolve_provider(kwargs, seams).filter(|provider| provider == OPENAI)
    else {
        return;
    };
    let Some(thinking_object) = thinking.and_then(object) else {
        return;
    };
    if !matches!(thinking_object.get(THINKING_TYPE), Some(Json::Text(kind)) if kind == ENABLED) {
        return;
    }
    let refused = seams
        .metadata
        .model_info(
            text_field(kwargs.fields.get(MODEL)),
            Some(provider.as_str()),
        )
        .is_some_and(|info| matches!(info.fields.get(SUPPORTS_REASONING), Some(Json::Bool(false))));
    if refused {
        return;
    }
    rewrite_model(kwargs, &provider);
    let auto_summary = auto_summary_enabled(settings.global, settings.environment);
    convert_effort(kwargs, thinking_object, auto_summary);
}

// _plain_effort_for_chat_target: the effort a chat target keeps once the bridge policy says it stays on chat
pub fn plain_effort(
    kwargs: &CompletionKwargs,
    thinking: Option<&Json>,
    seams: &ReasoningSeams<'_>,
) -> Result<Option<String>, ReasoningRouteError> {
    let Some((reasoning, effort, model)) = plain_target(kwargs, thinking) else {
        return Ok(None);
    };
    let Some(target) = seams.provider.resolve(&ProviderLookup {
        model: Some(model),
        custom_llm_provider: text_field(kwargs.fields.get(CUSTOM_LLM_PROVIDER)),
        api_base: text_field(kwargs.fields.get(API_BASE)),
        api_key: text_field(kwargs.fields.get(API_KEY)),
    }) else {
        return Ok(None);
    };
    if target.provider == LITELLM_PROXY {
        return Ok(None);
    }
    let outcome = seams.bridge.bridge(&BridgeRequest {
        model: &target.model,
        custom_llm_provider: &target.provider,
        web_search_options: kwargs
            .fields
            .get(WEB_SEARCH_OPTIONS)
            .filter(|value| matches!(value, Json::Object(_))),
        tools: kwargs.fields.get(TOOLS).and_then(|value| match value {
            Json::List(tools) => Some(tools.as_slice()),
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_) => None,
        }),
        reasoning_effort: reasoning,
        api_base: target.api_base.as_deref(),
    })?;
    if outcome.mode.as_deref() == Some(RESPONSES_MODE) {
        return Ok(None);
    }
    Ok(Some(effort.to_owned()))
}
