use std::collections::{BTreeMap, BTreeSet};

use serde_json::value::RawValue;

// The OpenAI sets follow openai 2.54.0, the version tools/oracle/litellm/uv.lock pins; rerank and Anthropic names come from LiteLLM's typed requests.
pub const CHAT_PARAMETER_NAMES: &[&str] = &[
    "audio",
    "frequency_penalty",
    "function_call",
    "functions",
    "logit_bias",
    "logprobs",
    "max_completion_tokens",
    "max_tokens",
    "messages",
    "metadata",
    "modalities",
    "model",
    "moderation",
    "n",
    "parallel_tool_calls",
    "prediction",
    "presence_penalty",
    "prompt_cache_key",
    "prompt_cache_options",
    "prompt_cache_retention",
    "reasoning_effort",
    "response_format",
    "safety_identifier",
    "seed",
    "service_tier",
    "stop",
    "store",
    "stream",
    "stream_options",
    "temperature",
    "thinking",
    "tool_choice",
    "tools",
    "top_logprobs",
    "top_p",
    "user",
    "verbosity",
    "web_search_options",
];

pub const TEXT_PARAMETER_NAMES: &[&str] = &[
    "best_of",
    "echo",
    "frequency_penalty",
    "logit_bias",
    "logprobs",
    "max_tokens",
    "model",
    "n",
    "presence_penalty",
    "prompt",
    "seed",
    "stop",
    "stream",
    "stream_options",
    "suffix",
    "temperature",
    "top_p",
    "user",
];

pub const EMBEDDING_PARAMETER_NAMES: &[&str] =
    &["dimensions", "encoding_format", "input", "model", "user"];

pub const TRANSCRIPTION_PARAMETER_NAMES: &[&str] = &[
    "chunking_strategy",
    "file",
    "include",
    "keywords",
    "known_speaker_names",
    "known_speaker_references",
    "language",
    "languages",
    "model",
    "prompt",
    "response_format",
    "stream",
    "temperature",
    "timestamp_granularities",
];

pub const RERANK_PARAMETER_NAMES: &[&str] = &[
    "documents",
    "instruction",
    "max_chunks_per_doc",
    "max_tokens_per_doc",
    "max_tokens_per_query",
    "model",
    "query",
    "rank_fields",
    "return_documents",
    "top_n",
    "truncate_prompt_tokens",
    "truncation_side",
];

pub const RESPONSES_PARAMETER_NAMES: &[&str] = &[
    "background",
    "context_management",
    "conversation",
    "include",
    "input",
    "instructions",
    "max_output_tokens",
    "max_tool_calls",
    "metadata",
    "model",
    "moderation",
    "parallel_tool_calls",
    "previous_response_id",
    "prompt",
    "prompt_cache_key",
    "prompt_cache_options",
    "prompt_cache_retention",
    "reasoning",
    "safety_identifier",
    "service_tier",
    "store",
    "stream",
    "stream_options",
    "temperature",
    "text",
    "tool_choice",
    "tools",
    "top_logprobs",
    "top_p",
    "truncation",
    "user",
];

pub const ANTHROPIC_PARAMETER_NAMES: &[&str] = &[
    "cache_control",
    "compaction",
    "container",
    "context_management",
    "inference_geo",
    "litellm_metadata",
    "max_tokens",
    "mcp_servers",
    "messages",
    "metadata",
    "model",
    "output_config",
    "output_format",
    "reasoning_effort",
    "safeguards",
    "speed",
    "stop_sequences",
    "stream",
    "system",
    "temperature",
    "thinking",
    "tool_choice",
    "tools",
    "top_k",
    "top_p",
];

pub const PROVIDER_CHAT_PARAMETER_NAMES: &[&str] = &["thinking"];

pub const CONTENT_PARAMETER_NAMES: &[&str] = &["input", "messages", "prompt", "system"];

pub const EXCLUDED_PARAMETER_NAMES: &[&str] = &["litellm_metadata", "metadata"];

pub type ModelParameters = BTreeMap<String, Box<RawValue>>;

#[must_use]
pub fn api_parameter_names() -> BTreeSet<&'static str> {
    let mut names = BTreeSet::new();
    for set in [
        CHAT_PARAMETER_NAMES,
        TEXT_PARAMETER_NAMES,
        EMBEDDING_PARAMETER_NAMES,
        TRANSCRIPTION_PARAMETER_NAMES,
        RERANK_PARAMETER_NAMES,
        RESPONSES_PARAMETER_NAMES,
        ANTHROPIC_PARAMETER_NAMES,
    ] {
        names.extend(set.iter().copied());
    }
    names.retain(|name| !EXCLUDED_PARAMETER_NAMES.contains(name));
    names
}

#[must_use]
pub fn logging_parameter_names() -> BTreeSet<&'static str> {
    let mut names = api_parameter_names();
    names.retain(|name| !CONTENT_PARAMETER_NAMES.contains(name));
    names
}

#[must_use]
pub fn logging_model_parameters(parameters: &ModelParameters) -> ModelParameters {
    let logging = logging_parameter_names();
    parameters
        .iter()
        .filter(|(name, _)| logging.contains(name.as_str()))
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect()
}

mod tests;
