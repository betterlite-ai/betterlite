use betterlite_prompts::arguments::json::{Json, Object};

use crate::model_urls::model_name::is_gemini_3_or_newer;

use super::error::GeminiOptionsError;

// LiteLLM advertises these names in this order, then appends the model-dependent groups below
pub const SUPPORTED_OPENAI_PARAMS: [&str; 22] = [
    "temperature",
    "top_p",
    "top_k",
    "max_tokens",
    "max_completion_tokens",
    "stream",
    "tools",
    "functions",
    "tool_choice",
    "response_format",
    "n",
    "stop",
    "extra_headers",
    "seed",
    "logprobs",
    "top_logprobs",
    "modalities",
    "audio",
    "parallel_tool_calls",
    "web_search_options",
    "include_server_side_tool_invocations",
    "service_tier",
];

pub const PENALTY_PARAMS: [&str; 2] = ["frequency_penalty", "presence_penalty"];
pub const THINKING_PARAMS: [&str; 2] = ["reasoning_effort", "thinking"];

// LiteLLM copies these request spellings onto the provider names before it maps the chat options
pub const MAPPED_SPECIAL_AUTH_PARAMS: [(&str, &str); 2] = [
    ("project", "vertex_project"),
    ("region_name", "vertex_location"),
];

// Source: https://cloud.google.com/vertex-ai/generative-ai/docs/learn/locations#available-regions
pub const EU_REGIONS: [&str; 10] = [
    "europe-central2",
    "europe-north1",
    "europe-southwest1",
    "europe-west1",
    "europe-west2",
    "europe-west3",
    "europe-west4",
    "europe-west6",
    "europe-west8",
    "europe-west9",
];

pub const US_REGIONS: [&str; 8] = [
    "us-central1",
    "us-east1",
    "us-east4",
    "us-east5",
    "us-south1",
    "us-west1",
    "us-west4",
    "us-west5",
];

// the preview model rejects penalties, and Gemini 3 dropped them, so both keep the short list
const UNSUPPORTED_PENALTY_MODELS: [&str; 1] = ["gemini-2.5-pro-preview-06-05"];

pub trait ReasoningSupport {
    fn supports_reasoning(&self, model: &str) -> bool;
}

#[must_use]
pub fn supports_penalty_parameters(model: &str) -> bool {
    // LiteLLM compares the model against the exclusion list as written, so a prefixed name is not excluded
    !is_gemini_3_or_newer(model) && !UNSUPPORTED_PENALTY_MODELS.contains(&model)
}

#[must_use]
pub fn supported_openai_params(model: &str, support: &dyn ReasoningSupport) -> Vec<&'static str> {
    let mut params = SUPPORTED_OPENAI_PARAMS.to_vec();
    if supports_penalty_parameters(model) {
        params.extend(PENALTY_PARAMS);
    }
    if is_gemini_3_or_newer(model) || support.supports_reasoning(model) {
        params.extend(THINKING_PARAMS);
    }
    params
}

// the request value wins over whatever the caller already put under the provider name
pub fn map_special_auth_params(
    non_default: &Object,
    mut optional: Object,
) -> Result<Object, GeminiOptionsError> {
    for (param, value) in non_default {
        let Some((name, mapped)) = MAPPED_SPECIAL_AUTH_PARAMS
            .iter()
            .find(|(name, _)| *name == param.as_str())
        else {
            continue;
        };
        match value {
            Json::Null => {}
            Json::Text(_) => {
                optional.insert((*mapped).to_owned(), value.clone());
            }
            Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
                return Err(GeminiOptionsError::FieldType {
                    param: (*name).to_owned(),
                    expected: "a string",
                });
            }
        }
    }
    Ok(optional)
}
