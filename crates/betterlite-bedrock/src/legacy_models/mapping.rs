use betterlite_openai::cohere_chat_v1;
use betterlite_prompts::arguments::json::{Json, Object};

use crate::legacy_models::family::Family;

pub const AI21_SUPPORTED_PARAMS: [&str; 4] = ["max_tokens", "temperature", "top_p", "stream"];
pub const LLAMA_SUPPORTED_PARAMS: [&str; 4] = ["max_tokens", "temperature", "top_p", "stream"];
pub const MISTRAL_SUPPORTED_PARAMS: [&str; 5] =
    ["max_tokens", "temperature", "top_p", "stop", "stream"];
pub const TITAN_SUPPORTED_PARAMS: [&str; 6] = [
    "max_tokens",
    "max_completion_tokens",
    "stop",
    "temperature",
    "top_p",
    "stream",
];
pub const ANTHROPIC_SUPPORTED_PARAMS: [&str; 6] = [
    "max_tokens",
    "max_completion_tokens",
    "temperature",
    "stop",
    "top_p",
    "stream",
];

const AI21_RENAMES: [(&str, &str); 4] = [
    ("max_tokens", "maxTokens"),
    ("temperature", "temperature"),
    ("top_p", "topP"),
    ("stream", "stream"),
];
const LLAMA_RENAMES: [(&str, &str); 4] = [
    ("max_tokens", "max_gen_len"),
    ("temperature", "temperature"),
    ("top_p", "top_p"),
    ("stream", "stream"),
];
const MISTRAL_RENAMES: [(&str, &str); 5] = [
    ("max_tokens", "max_tokens"),
    ("temperature", "temperature"),
    ("top_p", "top_p"),
    ("stop", "stop"),
    ("stream", "stream"),
];
const TITAN_RENAMES: [(&str, &str); 5] = [
    ("max_tokens", "maxTokenCount"),
    ("max_completion_tokens", "maxTokenCount"),
    ("temperature", "temperature"),
    ("top_p", "topP"),
    ("stream", "stream"),
];
const ANTHROPIC_RENAMES: [(&str, &str); 5] = [
    ("max_tokens", "max_tokens_to_sample"),
    ("max_completion_tokens", "max_tokens_to_sample"),
    ("temperature", "temperature"),
    ("top_p", "top_p"),
    ("stop", "stop_sequences"),
];

#[must_use]
pub const fn supported_openai_params(family: Family) -> &'static [&'static str] {
    match family {
        Family::Ai21 => &AI21_SUPPORTED_PARAMS,
        Family::Cohere => &cohere_chat_v1::SUPPORTED_OPENAI_PARAMS,
        Family::Llama => &LLAMA_SUPPORTED_PARAMS,
        Family::Mistral => &MISTRAL_SUPPORTED_PARAMS,
        Family::Titan => &TITAN_SUPPORTED_PARAMS,
        Family::Anthropic => &ANTHROPIC_SUPPORTED_PARAMS,
    }
}

pub struct MappingRequest<'values> {
    pub family: Family,
    pub non_default: &'values Object,
    pub optional: Object,
    pub model: &'values str,
    // The method argument every LiteLLM mapper receives and never reads; only the global switch filters.
    pub drop_params: bool,
    pub global_drop_params: bool,
}

#[must_use]
pub fn map_openai_params(request: MappingRequest<'_>) -> Object {
    match request.family {
        Family::Ai21 => renamed(request.non_default, request.optional, &AI21_RENAMES),
        Family::Cohere => cohere_chat_v1::map_openai_params(request.non_default, request.optional),
        Family::Llama => renamed(request.non_default, request.optional, &LLAMA_RENAMES),
        Family::Mistral => renamed(request.non_default, request.optional, &MISTRAL_RENAMES),
        Family::Titan => titan(request),
        Family::Anthropic => anthropic(request),
    }
}

fn renamed(non_default: &Object, mut optional: Object, renames: &[(&str, &str)]) -> Object {
    for (name, value) in non_default {
        let rename = renames.iter().find(|(source, _)| source == name);
        if let Some((_, target)) = rename {
            optional.insert((*target).to_owned(), value.clone());
        }
    }
    optional
}

// Both token aliases write one key, so the alias later in the input wins.
fn titan(request: MappingRequest<'_>) -> Object {
    let mut optional = renamed(request.non_default, request.optional, &TITAN_RENAMES);
    if let Some(stop) = request.non_default.get("stop") {
        let mut supported = Object::from_iter([("stop".to_owned(), stop.clone())]);
        map_and_modify_arg(
            &mut supported,
            request.global_drop_params,
            "bedrock",
            request.model,
            stop,
        );
        if let Some(filtered) = supported.get("stop") {
            optional.insert("stopSequences".to_owned(), filtered.clone());
        }
    }
    optional
}

// An unset or false stream leaves whatever optional_params already carries.
fn anthropic(request: MappingRequest<'_>) -> Object {
    let mut optional = renamed(request.non_default, request.optional, &ANTHROPIC_RENAMES);
    if matches!(request.non_default.get("stream"), Some(Json::Bool(true))) {
        optional.insert("stream".to_owned(), Json::Bool(true));
    }
    optional
}

pub fn map_and_modify_arg(
    supported: &mut Object,
    global_drop_params: bool,
    provider: &str,
    model: &str,
    stop: &Json,
) {
    let filtering = supported.contains_key("stop")
        && global_drop_params
        && provider == "bedrock"
        && model.contains("amazon");
    if !filtering {
        return;
    }
    let kept = match stop {
        Json::List(items) => items
            .iter()
            .filter(|item| retained(item))
            .cloned()
            .collect(),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_) => {
            Vec::new()
        }
    };
    supported.insert("stop".to_owned(), Json::List(kept));
}

// Python's ^(\|+|User:)$ keeps pipes and the exact User: marker, with $ matching before one final newline.
fn retained(item: &Json) -> bool {
    let Json::Text(text) = item else {
        return false;
    };
    let body = text.strip_suffix('\n').unwrap_or(text);
    body == "User:" || (!body.is_empty() && body.bytes().all(|byte| byte == b'|'))
}
