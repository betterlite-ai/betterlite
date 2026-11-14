use serde_json::Value;

use super::{
    EFFORT_KWARG_KEYS, EXTRA_BODY_CONSUMED_PARAMS, FireworksCapabilities, NIM_VLLM_STRIP_PARAMS,
    effort_from_template_kwargs, translate_guided_params,
};
use crate::openai_base_config::Object;

// LiteLLM's map_extra_body_params: strip NIM/vLLM keys, promote the three translations, keep the rest.
#[must_use]
pub async fn map_extra_body_params<P>(
    capabilities: &mut P,
    model: &str,
    optional_params: &Object,
) -> Object
where
    P: FireworksCapabilities,
{
    let Some(extra_body) = optional_params.get("extra_body").and_then(Value::as_object) else {
        return optional_params.clone();
    };
    log_stripped(extra_body, model);
    let promoted = promoted_params(capabilities, extra_body, optional_params, model).await;
    if extra_body.contains_key("response_format") && optional_params.contains_key("response_format")
    {
        tracing::debug!(
            "fireworks_ai dropping extra_body.response_format; the top-level response_format takes precedence"
        );
    }
    let remaining = remaining_extra_body(extra_body, optional_params);
    let mut result = base_params(optional_params);
    for (key, value) in promoted {
        result.insert(key.to_owned(), value);
    }
    if !remaining.is_empty() {
        result.insert(String::from("extra_body"), Value::Object(remaining));
    }
    result
}

fn log_stripped(extra_body: &Object, model: &str) {
    let stripped: Vec<&str> = NIM_VLLM_STRIP_PARAMS
        .iter()
        .copied()
        .filter(|name| extra_body.contains_key(*name))
        .collect();
    if !stripped.is_empty() {
        tracing::debug!(
            ?stripped,
            model,
            "fireworks_ai does not support NIM/vLLM params; dropping them from the request"
        );
    }
}

async fn promoted_params<P>(
    capabilities: &mut P,
    extra_body: &Object,
    optional_params: &Object,
    model: &str,
) -> Vec<(&'static str, Value)>
where
    P: FireworksCapabilities,
{
    let mut promoted = Vec::new();
    if let Some(value) = translate_truncate_tokens(extra_body, optional_params) {
        promoted.push(("prompt_truncate_len", value));
    }
    if let Some(value) =
        translate_template_kwargs(capabilities, extra_body, optional_params, model).await
    {
        promoted.push(("reasoning_effort", value));
    }
    if let Some(value) = translate_guided_params(extra_body, optional_params) {
        promoted.push(("response_format", value));
    }
    promoted
}

fn translate_truncate_tokens(extra_body: &Object, optional_params: &Object) -> Option<Value> {
    let value = extra_body.get("truncate_prompt_tokens")?;
    if value.is_null() {
        return None;
    }
    if extra_body.contains_key("prompt_truncate_len")
        || optional_params.contains_key("prompt_truncate_len")
    {
        tracing::debug!(
            "fireworks_ai ignoring truncate_prompt_tokens; explicit prompt_truncate_len takes precedence"
        );
        return None;
    }
    Some(value.clone())
}

fn log_unknown_kwargs(kwargs: &Object, model: &str) {
    let other_keys: Vec<&str> = kwargs
        .keys()
        .filter(|key| !EFFORT_KWARG_KEYS.contains(&key.as_str()))
        .map(String::as_str)
        .collect();
    if !other_keys.is_empty() {
        tracing::debug!(
            ?other_keys,
            model,
            "fireworks_ai does not support chat_template_kwargs keys; dropping them"
        );
    }
}

async fn translate_template_kwargs<P>(
    capabilities: &mut P,
    extra_body: &Object,
    optional_params: &Object,
    model: &str,
) -> Option<Value>
where
    P: FireworksCapabilities,
{
    let raw = extra_body
        .get("chat_template_kwargs")
        .filter(|value| !value.is_null())?;
    let Value::Object(kwargs) = raw else {
        tracing::debug!(
            model,
            "fireworks_ai dropping chat_template_kwargs; expected an object"
        );
        return None;
    };
    log_unknown_kwargs(kwargs, model);
    let explicit = ["reasoning_effort", "thinking"]
        .iter()
        .any(|key| optional_params.contains_key(*key) || extra_body.contains_key(*key));
    if explicit {
        tracing::debug!(
            "fireworks_ai ignoring chat_template_kwargs; explicit reasoning_effort/thinking takes precedence"
        );
        return None;
    }
    let effort = effort_from_template_kwargs(kwargs)?;
    if !capabilities.supports_reasoning(model).await {
        tracing::debug!(
            model,
            "fireworks_ai model does not support reasoning; dropping chat_template_kwargs effort keys"
        );
        return None;
    }
    Some(effort)
}

fn remaining_extra_body(extra_body: &Object, optional_params: &Object) -> Object {
    extra_body
        .iter()
        .filter(|(key, _)| {
            !EXTRA_BODY_CONSUMED_PARAMS.contains(&key.as_str())
                && (key.as_str() != "response_format"
                    || !optional_params.contains_key("response_format"))
        })
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}

fn base_params(optional_params: &Object) -> Object {
    optional_params
        .iter()
        .filter(|(key, _)| key.as_str() != "extra_body")
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}
