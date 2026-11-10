use std::collections::BTreeSet;

use serde::Deserialize;

use crate::identity::provider_id::ProviderId;
use crate::params::CallType;

mod tests;

// OpenAIGPTConfig.get_supported_openai_params base list; response_format and user are appended per model.
const BASE_PARAMETER_NAMES: &[&str] = &[
    "frequency_penalty",
    "logit_bias",
    "logprobs",
    "top_logprobs",
    "max_tokens",
    "max_completion_tokens",
    "modalities",
    "prediction",
    "n",
    "presence_penalty",
    "seed",
    "stop",
    "stream",
    "stream_options",
    "temperature",
    "top_p",
    "tools",
    "tool_choice",
    "function_call",
    "functions",
    "max_retries",
    "extra_headers",
    "parallel_tool_calls",
    "audio",
    "web_search_options",
    "service_tier",
    "safety_identifier",
    "prompt_cache_key",
    "prompt_cache_retention",
    "store",
];

const GPT_5_SEARCH_PARAMETER_NAMES: &[&str] = &[
    "max_tokens",
    "max_completion_tokens",
    "stream",
    "stream_options",
    "web_search_options",
    "service_tier",
    "safety_identifier",
    "response_format",
    "user",
    "store",
    "verbosity",
    "max_retries",
    "extra_headers",
];

const GPT_5_UNSUPPORTED_NAMES: &[&str] = &[
    "presence_penalty",
    "frequency_penalty",
    "stop",
    "logit_bias",
    "modalities",
    "prediction",
    "audio",
    "web_search_options",
];

const O_SERIES_UNSUPPORTED_NAMES: &[&str] = &[
    "logprobs",
    "top_p",
    "presence_penalty",
    "frequency_penalty",
    "top_logprobs",
];

const O_SERIES_TOOL_PARAMETER_NAMES: &[&str] =
    &["tools", "tool_choice", "function_call", "functions"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpenAiConfigTag {
    Gpt,
    Gpt5,
    OSeries,
    UnknownModel,
}

// A capability the caller's catalog lookup resolved for the model; an absent member is unsupported.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpenAiCapability {
    CatalogModel,
    FunctionCalling,
    ParallelToolCalls,
    ResponseSchema,
    ToolChoice,
    NoneReasoningEffort,
}

#[derive(Debug, Clone, Copy)]
pub struct ModelProjection<'model> {
    pub model: &'model str,
    pub capabilities: &'model BTreeSet<OpenAiCapability>,
}

#[derive(Debug, Clone, Copy)]
pub struct CapabilityQuery<'request> {
    pub provider: ProviderId,
    pub call_type: CallType,
    pub config: OpenAiConfigTag,
    pub model: ModelProjection<'request>,
    pub base_model: Option<ModelProjection<'request>>,
}

// Absent is a provider whose adapter betterlite has not built, which is not an empty supported list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParameterCapabilities {
    Listed(Vec<&'static str>),
    Absent,
}

#[must_use]
pub fn supported_parameters(query: &CapabilityQuery<'_>) -> ParameterCapabilities {
    if query.provider != ProviderId::Openai || query.call_type == CallType::Transcription {
        return ParameterCapabilities::Absent;
    }
    let mut names = config_parameters(query.config, query.model);
    if query.call_type == CallType::ChatCompletion
        && let Some(base) = query
            .base_model
            .filter(|base| base.model != query.model.model)
    {
        for name in config_parameters(query.config, base) {
            if !names.contains(&name) {
                names.push(name);
            }
        }
    }
    ParameterCapabilities::Listed(names)
}

fn config_parameters(
    config: OpenAiConfigTag,
    projection: ModelProjection<'_>,
) -> Vec<&'static str> {
    match config {
        OpenAiConfigTag::Gpt => base_parameters(projection),
        OpenAiConfigTag::UnknownModel => {
            let mut names = base_parameters(projection);
            names.push("reasoning_effort");
            names
        }
        OpenAiConfigTag::OSeries => o_series_parameters(projection),
        OpenAiConfigTag::Gpt5 => gpt_5_parameters(projection),
    }
}

fn base_parameters(projection: ModelProjection<'_>) -> Vec<&'static str> {
    let mut names = BASE_PARAMETER_NAMES.to_vec();
    if projection.model != "gpt-3.5-turbo-16k" && projection.model != "gpt-4" {
        names.push("response_format");
    }
    if projection
        .capabilities
        .contains(&OpenAiCapability::CatalogModel)
    {
        names.push("user");
    }
    names
}

fn o_series_parameters(projection: ModelProjection<'_>) -> Vec<&'static str> {
    let mut names = base_parameters(projection);
    names.push("reasoning_effort");
    let gates: [(OpenAiCapability, &[&str]); 3] = [
        (
            OpenAiCapability::FunctionCalling,
            O_SERIES_TOOL_PARAMETER_NAMES,
        ),
        (
            OpenAiCapability::ParallelToolCalls,
            &["parallel_tool_calls"],
        ),
        (OpenAiCapability::ResponseSchema, &["response_format"]),
    ];
    for (capability, excluded) in gates {
        if !projection.capabilities.contains(&capability) {
            names.retain(|name| !excluded.contains(name));
        }
    }
    names.retain(|name| !O_SERIES_UNSUPPORTED_NAMES.contains(name));
    names
}

fn gpt_5_parameters(projection: ModelProjection<'_>) -> Vec<&'static str> {
    if projection.model.contains("gpt-5") && projection.model.contains("search") {
        return GPT_5_SEARCH_PARAMETER_NAMES.to_vec();
    }
    let mut names = base_parameters(projection);
    names.extend(["reasoning_effort", "verbosity"]);
    if !projection
        .capabilities
        .contains(&OpenAiCapability::ToolChoice)
    {
        names.retain(|name| *name != "tool_choice");
    }
    names.retain(|name| !GPT_5_UNSUPPORTED_NAMES.contains(name));
    if !projection
        .capabilities
        .contains(&OpenAiCapability::NoneReasoningEffort)
    {
        names.retain(|name| !matches!(*name, "logprobs" | "top_logprobs" | "top_p"));
    }
    names
}
