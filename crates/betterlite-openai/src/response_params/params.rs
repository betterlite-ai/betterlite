use std::collections::BTreeSet;

use betterlite_models::identity::provider_id::ProviderId;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::capabilities::CUSTOM_LLM_PROVIDER;
use crate::openai_base_config::Object;

const TRANSPORT_PARAMS: [&str; 4] = ["extra_headers", "extra_query", "extra_body", "timeout"];
const REQUEST_PARAMS: [&str; 30] = [
    "input",
    "model",
    "include",
    "instructions",
    "max_output_tokens",
    "metadata",
    "parallel_tool_calls",
    "previous_response_id",
    "reasoning",
    "store",
    "background",
    "stream",
    "temperature",
    "text",
    "tool_choice",
    "tools",
    "top_p",
    "truncation",
    "user",
    "service_tier",
    "safety_identifier",
    "prompt",
    "max_tool_calls",
    "prompt_cache_key",
    "prompt_cache_retention",
    "prompt_cache_options",
    "stream_options",
    "top_logprobs",
    "partial_images",
    "context_management",
];

#[must_use]
pub fn supported_openai_params() -> BTreeSet<&'static str> {
    let mut params = BTreeSet::new();
    params.extend(TRANSPORT_PARAMS);
    params.extend(REQUEST_PARAMS);
    params
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResponsesParams {
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "serde_with::rust::double_option"
    )]
    pub input: Option<Option<Value>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "serde_with::rust::double_option"
    )]
    pub reasoning: Option<Option<Value>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "serde_with::rust::double_option"
    )]
    pub text: Option<Option<Value>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "serde_with::rust::double_option"
    )]
    pub tool_choice: Option<Option<Value>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "serde_with::rust::double_option"
    )]
    pub max_output_tokens: Option<Option<Value>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "serde_with::rust::double_option"
    )]
    pub temperature: Option<Option<Value>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "serde_with::rust::double_option"
    )]
    pub top_p: Option<Option<Value>>,
    // the Responses parameters betterlite does not model, which LiteLLM copies untouched
    #[serde(flatten)]
    pub passthrough: Object,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResponsesOptions {
    #[serde(default = "default_provider")]
    pub provider: ProviderId,
    pub model: String,
    #[serde(default)]
    pub params: ResponsesParams,
}

const fn default_provider() -> ProviderId {
    CUSTOM_LLM_PROVIDER
}
