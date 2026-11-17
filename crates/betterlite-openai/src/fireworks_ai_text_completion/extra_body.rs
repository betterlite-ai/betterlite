use serde_json::Value;

use crate::fireworks_ai_chat_params::{
    EFFORT_KWARG_KEYS, FireworksCapabilities, effort_from_template_kwargs, translate_guided_params,
};
use crate::openai_base_config::Object;

// LiteLLM's _TEXT_COMPLETION_STRIP_PARAMS: the two prompt keys plus B6071's NIM/vLLM list, kept sorted.
pub const STRIP_PARAMS: [&str; 18] = [
    "add_generation_prompt",
    "add_special_tokens",
    "allowed_token_ids",
    "bad_words",
    "best_of",
    "continue_final_message",
    "detokenize",
    "guided_decoding_backend",
    "guided_regex",
    "include_reasoning",
    "include_stop_str_in_output",
    "nvext",
    "prompt_truncate_len",
    "skip_special_tokens",
    "spaces_between_special_tokens",
    "stop_token_ids",
    "truncate_prompt_tokens",
    "use_beam_search",
];

const MOVED_KEYS: [&str; 3] = ["response_format", "reasoning_effort", "thinking"];
const REASONING_KEYS: [&str; 2] = ["reasoning_effort", "thinking"];
const GUIDED_KEYS: [&str; 3] = ["guided_json", "guided_grammar", "guided_choice"];
const DROPPED_KEYS: [&str; 4] = [
    "extra_body",
    "response_format",
    "reasoning_effort",
    "thinking",
];

// LiteLLM's map_extra_body_params: strip, move the native params, translate chat_template_kwargs, then guided decoding.
#[must_use]
pub async fn map_extra_body_params<P>(
    capabilities: &mut P,
    model: &str,
    optional_params: &Object,
) -> Object
where
    P: FireworksCapabilities,
{
    let stripped = stripped_body(optional_params, model);
    let moved = moved_body(&stripped, optional_params);
    let effort = translated_kwargs(capabilities, &moved, optional_params, model).await;
    let guided = translate_guided_params(&effort, optional_params);
    let mut final_body: Object = effort
        .iter()
        .filter(|(key, _)| !GUIDED_KEYS.contains(&key.as_str()))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    if let Some(format) = guided {
        final_body.insert(String::from("response_format"), format);
    }
    let mut base: Object = optional_params
        .iter()
        .filter(|(key, _)| !DROPPED_KEYS.contains(&key.as_str()))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    if !final_body.is_empty() {
        base.insert(String::from("extra_body"), Value::Object(final_body));
    }
    base
}

fn stripped_body(optional_params: &Object, model: &str) -> Object {
    let Some(Value::Object(body)) = optional_params.get("extra_body") else {
        return Object::new();
    };
    let dropped: Vec<&str> = body
        .keys()
        .map(String::as_str)
        .filter(|key| STRIP_PARAMS.contains(key))
        .collect();
    log_stripped(&dropped, model);
    body.iter()
        .filter(|(key, _)| !STRIP_PARAMS.contains(&key.as_str()))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}

// the debug event is not part of the body, so no test can tell when it fires
#[cfg_attr(test, mutants::skip)]
fn log_stripped(dropped: &[&str], model: &str) {
    if !dropped.is_empty() {
        let mut names = dropped.to_vec();
        names.sort_unstable();
        tracing::debug!(
            ?names,
            model,
            "fireworks_ai does not support NIM/vLLM params; dropping them from the request"
        );
    }
}

fn moved_body(body: &Object, optional_params: &Object) -> Object {
    let mut moved = body.clone();
    for key in MOVED_KEYS {
        let Some(value) = optional_params.get(key).filter(|value| !value.is_null()) else {
            continue;
        };
        if moved.contains_key(key) {
            tracing::debug!(
                key,
                "fireworks_ai overriding an extra_body value with the top-level value"
            );
        }
        moved.insert(key.to_owned(), value.clone());
    }
    moved
}

async fn translated_kwargs<P>(
    capabilities: &mut P,
    body: &Object,
    optional_params: &Object,
    model: &str,
) -> Object
where
    P: FireworksCapabilities,
{
    let Some(raw) = body
        .get("chat_template_kwargs")
        .filter(|value| !value.is_null())
    else {
        return body.clone();
    };
    let mut result = body.clone();
    result.remove("chat_template_kwargs");
    let Value::Object(kwargs) = raw else {
        tracing::debug!(
            model,
            "fireworks_ai dropping chat_template_kwargs; expected an object"
        );
        return result;
    };
    log_unknown_kwargs(kwargs, model);
    let Some(effort) = effort_from_template_kwargs(kwargs) else {
        return result;
    };
    let explicit = REASONING_KEYS
        .iter()
        .any(|key| result.contains_key(*key) || optional_params.contains_key(*key));
    if explicit {
        tracing::debug!(
            "fireworks_ai ignoring chat_template_kwargs; explicit reasoning_effort/thinking takes precedence"
        );
        return result;
    }
    if !capabilities.supports_reasoning(model).await {
        tracing::debug!(
            model,
            "fireworks_ai model does not support reasoning; dropping chat_template_kwargs effort keys"
        );
        return result;
    }
    result.insert(String::from("reasoning_effort"), effort);
    result
}

// the debug event is not part of the body, so no test can tell when it fires
#[cfg_attr(test, mutants::skip)]
fn log_unknown_kwargs(kwargs: &Object, model: &str) {
    let mut unknown: Vec<&str> = kwargs
        .keys()
        .map(String::as_str)
        .filter(|key| !EFFORT_KWARG_KEYS.contains(key))
        .collect();
    unknown.sort_unstable();
    if !unknown.is_empty() {
        tracing::debug!(
            ?unknown,
            model,
            "fireworks_ai does not support chat_template_kwargs keys; dropping them"
        );
    }
}
