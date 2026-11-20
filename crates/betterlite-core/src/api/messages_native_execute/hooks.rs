use std::sync::Arc;

use betterlite_anthropic::message_replay::tool_ids::sanitize_tool_use_ids;
use betterlite_anthropic::message_replay::{BlockStrip, strip_blocks};
use betterlite_anthropic::web_replay::flatten_web_search_results;
use betterlite_models::decisions::Json;
use betterlite_prompts::arguments::json::Json as PromptJson;
use indexmap::IndexMap;

use super::entry::MessagesEntryRequest;
use super::error::MessagesExecuteError;
use super::seams::{CacheControlOutcome, CacheControlRequest, MessagesEntrySeams, PreRequestHook};
use crate::api::messages_native_transform::error::TransformError;
use crate::api::messages_reasoning_route::seams::ProviderLookup;

pub(super) fn litellm_params(provider: Option<&str>) -> Json {
    let mut fields: IndexMap<String, Json> = IndexMap::new();
    fields.insert(
        "custom_llm_provider".to_owned(),
        match provider {
            Some(name) => Json::Text(name.to_owned()),
            None => Json::Null,
        },
    );
    Json::Object(fields)
}

// request_kwargs.get("litellm_params", {}).get("custom_llm_provider"), including its AttributeError
pub(super) fn provider_from_hook_map(
    hook_map: &IndexMap<String, Json>,
) -> Result<Option<String>, MessagesExecuteError> {
    let Some(value) = hook_map.get("litellm_params") else {
        return Ok(None);
    };
    let Json::Object(params) = value else {
        return Err(MessagesExecuteError::MissingAttribute {
            object_type: value_type_name(value).to_owned(),
        });
    };
    match params.get("custom_llm_provider") {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(name)) if name.is_empty() => Ok(None),
        Some(Json::Text(name)) => Ok(Some(name.clone())),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(MessagesExecuteError::FieldType {
                field: "litellm_params.custom_llm_provider",
                expected: "a string",
            })
        }
    }
}

fn value_type_name(value: &Json) -> &'static str {
    match value {
        Json::Null => "NoneType",
        Json::Bool(_) => "bool",
        Json::Number(number) if number.is_i64() || number.is_u64() => "int",
        Json::Number(_) => "float",
        Json::Text(_) => "str",
        Json::List(_) => "list",
        Json::Object(_) => "dict",
    }
}

// tools and stream are the hook map's two typed entries: an explicit null clears them, a wrong type stops
pub(super) fn pop_typed_arguments(
    hook_map: &mut IndexMap<String, Json>,
    tools_original: Option<Vec<Json>>,
    stream_original: Option<bool>,
) -> Result<(Option<Vec<Json>>, Option<bool>), MessagesExecuteError> {
    let tools = match hook_map.shift_remove("tools") {
        None => tools_original,
        Some(Json::Null) => None,
        Some(Json::List(items)) => Some(items),
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_)) => {
            return Err(MessagesExecuteError::FieldType {
                field: "tools",
                expected: "a list",
            });
        }
    };
    let stream = match hook_map.shift_remove("stream") {
        None => stream_original,
        Some(Json::Null) => None,
        Some(Json::Bool(value)) => Some(value),
        Some(Json::Number(_) | Json::Text(_) | Json::List(_) | Json::Object(_)) => {
            return Err(MessagesExecuteError::FieldType {
                field: "stream",
                expected: "a boolean",
            });
        }
    };
    Ok((tools, stream))
}

// An explicit null in the hook map is the named argument's None, not a kept original
pub(super) fn pop_named(
    hook_map: &mut IndexMap<String, Json>,
    name: &str,
    original: Option<Json>,
) -> Option<Json> {
    match hook_map.shift_remove(name) {
        None => original,
        Some(Json::Null) => None,
        Some(value) => Some(value),
    }
}

// B571, then B572, over the prompt types their implementations own
pub(super) fn sanitize_messages(messages: &[Json]) -> Result<Vec<Json>, MessagesExecuteError> {
    let encoded = serde_json::to_vec(messages).map_err(TransformError::from)?;
    let prompt_messages: Vec<PromptJson> =
        serde_json::from_slice(&encoded).map_err(TransformError::from)?;
    let stripped = strip_blocks(&prompt_messages, BlockStrip::EmptyContent);
    let with_ids = sanitize_tool_use_ids(&stripped);
    let reencoded = serde_json::to_vec(&with_ids).map_err(TransformError::from)?;
    let contract: Vec<Json> = serde_json::from_slice(&reencoded).map_err(TransformError::from)?;
    Ok(flatten_web_search_results(&contract))
}

// AnthropicCacheControlHook.maybe_inject_cache_control runs first and may replace messages or system
pub(super) fn cache_stage(
    seams: &MessagesEntrySeams<'_>,
    request: &MessagesEntryRequest,
    messages: &[Json],
    kwargs: &mut IndexMap<String, Json>,
) -> Result<CacheControlOutcome, MessagesExecuteError> {
    seams.cache.maybe_inject_cache_control(CacheControlRequest {
        messages,
        system: request.system.as_ref(),
        kwargs,
        model: &request.model,
        custom_llm_provider: request.custom_llm_provider.as_deref(),
        tools: request.tools.as_deref(),
        api_base: request.api_base.as_deref(),
    })
}

// The hook map every CustomLogger sees: the named arguments, litellm_params, then the caller's extras
pub(super) fn build_hook_map(
    request: &MessagesEntryRequest,
    provider: Option<&str>,
    stream: bool,
    kwargs: &IndexMap<String, Json>,
) -> IndexMap<String, Json> {
    let mut hook_map: IndexMap<String, Json> = IndexMap::new();
    hook_map.insert(
        "tools".to_owned(),
        match request.tools.as_deref() {
            Some(list) => Json::List(list.to_vec()),
            None => Json::Null,
        },
    );
    hook_map.insert("stream".to_owned(), Json::Bool(stream));
    hook_map.insert("litellm_params".to_owned(), litellm_params(provider));
    hook_map.insert(
        "tool_choice".to_owned(),
        request.tool_choice.clone().unwrap_or(Json::Null),
    );
    for (key, value) in kwargs {
        hook_map.insert(key.clone(), value.clone());
    }
    hook_map
}

// A None return keeps the map, a returned map replaces it, and an error stops before any provider call
pub(super) async fn run_pre_request_hooks(
    hooks: &[Arc<dyn PreRequestHook>],
    model: &str,
    messages: &[Json],
    mut hook_map: IndexMap<String, Json>,
) -> Result<IndexMap<String, Json>, MessagesExecuteError> {
    for hook in hooks {
        let modified = hook.pre_request(model, messages, &mut hook_map).await?;
        if let Some(replacement) = modified {
            hook_map = replacement;
        }
    }
    Ok(hook_map)
}

// B174's get_llm_provider(model=model), called when no provider is set yet; LiteLLM passes no api_base or key
pub(super) fn resolve_provider(
    request: &MessagesEntryRequest,
    seams: &MessagesEntrySeams<'_>,
) -> Option<String> {
    let lookup = ProviderLookup {
        model: Some(request.model.as_str()),
        custom_llm_provider: None,
        api_base: None,
        api_key: None,
    };
    seams
        .provider_resolution
        .resolve(&lookup)
        .map(|target| target.provider)
}
