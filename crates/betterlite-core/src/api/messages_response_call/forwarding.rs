use betterlite_models::controls::json_value::JsonValue;
use betterlite_prompts::arguments::json::{Json, Object};

use crate::api::response_id_traversal::response_id::transcode;
use crate::api::responses_create::error::CreateError;
use crate::api::responses_provider_selection::ParameterMap;

use super::error::MessagesResponseCallError;

pub(super) const ANTHROPIC_MESSAGES: &str = "anthropic_messages";
pub(super) const CUSTOM_LLM_PROVIDER: &str = "custom_llm_provider";
pub(super) const INCLUDE: &str = "include";
pub(super) const PROMPT_CACHE_KEY: &str = "prompt_cache_key";

// The custom_llm_provider kwarg when it is a string; LiteLLM only treats a string as the provider and infers otherwise.
pub(super) fn forwarded_provider(
    forwarded: &ParameterMap,
) -> Result<Option<&str>, MessagesResponseCallError> {
    match forwarded.get(CUSTOM_LLM_PROVIDER) {
        None | Some(JsonValue::Null) => Ok(None),
        Some(JsonValue::Text(provider)) => Ok(Some(provider)),
        Some(
            JsonValue::Bool(_) | JsonValue::Number(_) | JsonValue::Array(_) | JsonValue::Object(_),
        ) => Err(MessagesResponseCallError::Build(CreateError::Field {
            field: CUSTOM_LLM_PROVIDER,
            expected: "a string",
        })),
    }
}

// The litellm kwargs forward verbatim unless the bridge already wrote that key; anthropic_messages never travels.
pub(super) fn forward(
    object: &mut Object,
    forwarded: &ParameterMap,
) -> Result<(), MessagesResponseCallError> {
    for (key, value) in forwarded {
        if key == ANTHROPIC_MESSAGES || matches!(value, JsonValue::Null) || object.contains_key(key)
        {
            continue;
        }
        let converted = transcode::<JsonValue, Json>(value).map_err(CreateError::from)?;
        object.insert(key.clone(), converted);
    }
    Ok(())
}

pub(super) fn forwarded_include(
    forwarded: &ParameterMap,
) -> Result<Option<Vec<String>>, MessagesResponseCallError> {
    let Some(value) = forwarded.get(INCLUDE) else {
        return Ok(None);
    };
    if matches!(value, JsonValue::Null) {
        return Ok(None);
    }
    let JsonValue::Array(items) = value else {
        return Err(MessagesResponseCallError::Build(CreateError::Field {
            field: INCLUDE,
            expected: "a list of strings",
        }));
    };
    let mut names = Vec::with_capacity(items.len());
    for item in items {
        let JsonValue::Text(name) = item else {
            return Err(MessagesResponseCallError::Build(CreateError::Field {
                field: INCLUDE,
                expected: "a list of strings",
            }));
        };
        names.push(name.clone());
    }
    Ok(Some(names))
}

// The deployment's include entries are appended to the bridge's, keeping the bridge's order and dropping repeats.
pub(super) fn merge_include(object: &mut Object, deployment: Option<&[String]>) {
    let Some(extras) = deployment else {
        return;
    };
    let Some(Json::List(bridge)) = object.get(INCLUDE) else {
        return;
    };
    let mut merged = bridge.clone();
    for item in extras {
        if !bridge
            .iter()
            .any(|existing| existing.as_str() == Some(item.as_str()))
        {
            merged.push(Json::Text(item.clone()));
        }
    }
    object.insert(String::from(INCLUDE), Json::List(merged));
}

// A forwarded prompt_cache_key is set after the loop, so it replaces the one metadata.user_id decoded.
pub(super) fn override_prompt_cache_key(
    object: &mut Object,
    forwarded: &ParameterMap,
) -> Result<(), MessagesResponseCallError> {
    let Some(value) = forwarded.get(PROMPT_CACHE_KEY) else {
        return Ok(());
    };
    let JsonValue::Text(key) = value else {
        if matches!(value, JsonValue::Null) {
            return Ok(());
        }
        return Err(MessagesResponseCallError::Build(CreateError::Field {
            field: PROMPT_CACHE_KEY,
            expected: "a string",
        }));
    };
    object.insert(String::from(PROMPT_CACHE_KEY), Json::Text(key.clone()));
    Ok(())
}
