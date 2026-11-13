use crate::model_urls::model_name::is_gemini_3_or_newer;

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
