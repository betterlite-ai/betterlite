use betterlite_models::call_helpers::headers::AdditionalHeaders;
use betterlite_models::controls::optional_params::OptionalParams;
use betterlite_models::decisions::Json;
use betterlite_router::routing_timeouts::seconds::Seconds;
use indexmap::IndexMap;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
pub type JsonObject = IndexMap<String, Json>;
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct CompletionArgs {
    pub model: Option<String>,
    pub messages: Vec<Json>,
    pub functions: Option<Vec<Json>>,
    pub function_call: Option<Json>,
    pub timeout: Option<Json>,
    pub temperature: Option<Json>,
    pub top_p: Option<Json>,
    #[serde(rename = "n")]
    pub choice_count: Option<u64>,
    pub stream: Option<bool>,
    pub stream_options: Option<JsonObject>,
    pub stop: Option<Json>,
    pub max_tokens: Option<u64>,
    pub max_completion_tokens: Option<u64>,
    pub modalities: Option<Vec<Json>>,
    pub prediction: Option<JsonObject>,
    pub audio: Option<JsonObject>,
    pub presence_penalty: Option<Json>,
    pub frequency_penalty: Option<Json>,
    pub logit_bias: Option<JsonObject>,
    pub user: Option<String>,
    pub reasoning_effort: Option<Json>,
    pub verbosity: Option<String>,
    pub response_format: Option<Json>,
    pub seed: Option<i64>,
    pub tools: Option<Vec<Json>>,
    pub tool_choice: Option<Json>,
    pub parallel_tool_calls: Option<bool>,
    pub logprobs: Option<bool>,
    pub top_logprobs: Option<u64>,
    pub web_search_options: Option<JsonObject>,
    #[serde(rename = "include_server_side_tool_invocations")]
    pub include_side_tool_invocations: Option<bool>,
    pub deployment_id: Option<String>,
    pub extra_headers: Option<AdditionalHeaders>,
    pub safety_identifier: Option<String>,
    pub service_tier: Option<String>,
    pub store: Option<bool>,
    pub prompt_cache_key: Option<String>,
    pub base_url: Option<String>,
    pub api_version: Option<String>,
    #[serde(skip_serializing)]
    pub api_key: Option<SecretString>,
    pub model_list: Option<Vec<JsonObject>>,
    pub thinking: Option<Json>,
    pub enable_json_schema_validation: Option<bool>,
    #[serde(flatten)]
    pub kwargs: JsonObject,
}
#[derive(Clone, Debug, Default)]
pub struct CompletionSettings {
    pub drop_params: Option<bool>,
    pub model_fallbacks: Option<Vec<Json>>,
    pub model_alias_map: BTreeMap<String, String>,
    pub num_retries: Option<i64>,
    pub request_timeout: Option<Seconds>,
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
    pub route_all_to_responses: bool,
    pub proxy_headers: AdditionalHeaders,
    pub optional: OptionalParams,
    pub flags: CompletionFlags,
    pub medium_thinking_budget: u32,
}
#[derive(Clone, Copy, Debug, Default)]
pub struct CompletionFlags {
    pub add_function_to_prompt: bool,
    pub disable_stop_sequence_limit: bool,
}
