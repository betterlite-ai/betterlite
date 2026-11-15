use betterlite_models::decisions::Json;
use betterlite_models::prompt_spec::{PromptInfo, PromptLiteLLMParams};
use serde::Deserialize;

pub const AMBIGUOUS_PROMPT_DATA_ERROR: &str = "litellm_params.prompt_id cannot be combined with prompt_data keyed by template name. Send a flat template, prompt_data={\"content\": \"...\", \"metadata\": {...}}, together with litellm_params.prompt_id, or send prompt_data={\"<template_id>\": {\"content\": \"...\", \"metadata\": {...}}} without litellm_params.prompt_id.";

#[derive(Clone, Debug, Deserialize)]
pub struct Prompt {
    pub prompt_id: String,
    pub litellm_params: PromptLiteLLMParams,
    pub prompt_info: Option<PromptInfo>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct PatchPromptRequest {
    pub litellm_params: Option<PromptLiteLLMParams>,
    pub prompt_info: Option<PromptInfo>,
}

#[must_use]
pub fn is_ambiguous_keyed_prompt_data(litellm_params: &PromptLiteLLMParams) -> bool {
    let Some(Json::Object(prompt_data)) = litellm_params.extra.get("prompt_data") else {
        return false;
    };
    litellm_params.prompt_id.is_some()
        && !prompt_data.is_empty()
        && !prompt_data.contains_key("content")
}
