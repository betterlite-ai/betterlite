use crate::params::{CallType, TYPED_PARAMETER_NAMES};

mod tests;

// The validated chat names: DEFAULT_CHAT_COMPLETION_PARAM_VALUES without the excluded, content and transport keys.
pub const CHAT_STANDARD_PARAMETER_NAMES: &[&str] = &[
    "functions",
    "function_call",
    "temperature",
    "top_p",
    "n",
    "stream",
    "stream_options",
    "stop",
    "max_tokens",
    "max_completion_tokens",
    "modalities",
    "prediction",
    "audio",
    "presence_penalty",
    "frequency_penalty",
    "logit_bias",
    "user",
    "response_format",
    "seed",
    "tools",
    "tool_choice",
    "logprobs",
    "top_logprobs",
    "parallel_tool_calls",
    "reasoning_effort",
    "verbosity",
    "thinking",
    "web_search_options",
    "include_server_side_tool_invocations",
    "service_tier",
    "safety_identifier",
    "prompt_cache_key",
    "prompt_cache_retention",
    "store",
    "metadata",
    "context_management",
];

pub const EMBEDDING_STANDARD_NAMES: &[&str] = &["dimensions", "encoding_format", "user"];

// Credentials, request controls and content never travel through the passthrough map to a provider.
pub const RESERVED_PARAMETER_NAMES: &[&str] = &[
    "additional_drop_params",
    "allowed_openai_params",
    "api_base",
    "api_key",
    "api_version",
    "base_url",
    "caching",
    "context_window_fallback_dict",
    "custom_llm_provider",
    "drop_params",
    "extra_headers",
    "fallbacks",
    "input",
    "litellm_metadata",
    "max_retries",
    "messages",
    "mock_delay",
    "mock_response",
    "mock_timeout",
    "model",
    "num_retries",
    "organization",
    "prompt",
    "stream_timeout",
    "system",
    "timeout",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParameterClass {
    Standard,
    ProviderPassthrough,
    Reserved,
}

#[must_use]
pub fn classify_parameter(name: &str, call_type: CallType) -> ParameterClass {
    if TYPED_PARAMETER_NAMES.contains(&name) || RESERVED_PARAMETER_NAMES.contains(&name) {
        return ParameterClass::Reserved;
    }
    let standard = match call_type {
        CallType::ChatCompletion | CallType::Transcription => CHAT_STANDARD_PARAMETER_NAMES,
        CallType::Embeddings => EMBEDDING_STANDARD_NAMES,
    };
    if standard.contains(&name) {
        return ParameterClass::Standard;
    }
    ParameterClass::ProviderPassthrough
}
