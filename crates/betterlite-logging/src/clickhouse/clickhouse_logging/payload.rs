use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rust_decimal::Decimal;
use serde::Deserialize;

// The fields spend_log_row_from_payload reads; ints and floats stay JSON because the converters apply LiteLLM's type checks
#[derive(Clone, Debug, Default, Deserialize)]
pub struct StandardLoggingPayload {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub trace_id: Option<String>,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub call_type: Option<String>,
    #[serde(default, with = "rust_decimal::serde::arbitrary_precision_option")]
    pub response_cost: Option<Decimal>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub custom_llm_provider: Option<String>,
    #[serde(default)]
    pub prompt_tokens: Option<Json>,
    #[serde(default)]
    pub completion_tokens: Option<Json>,
    #[serde(default)]
    pub total_tokens: Option<Json>,
    #[serde(default, rename = "startTime")]
    pub start_time: Option<Json>,
    #[serde(default, rename = "endTime")]
    pub end_time: Option<Json>,
    #[serde(default, rename = "completionStartTime")]
    pub completion_start_time: Option<Json>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub model_id: Option<String>,
    #[serde(default)]
    pub model_group: Option<String>,
    #[serde(default)]
    pub api_base: Option<String>,
    #[serde(default)]
    pub cache_hit: Option<bool>,
    #[serde(default)]
    pub request_tags: Option<Json>,
    #[serde(default)]
    pub end_user: Option<String>,
    #[serde(default)]
    pub messages: Option<Json>,
    #[serde(default)]
    pub response: Option<Json>,
    #[serde(default)]
    pub error_str: Option<String>,
    #[serde(default)]
    pub metadata: Option<IndexMap<String, Json>>,
    #[serde(default)]
    pub hidden_params: Option<IndexMap<String, Json>>,
}

// The callback kwargs _log reads, including the request metadata the session id and traceparent searches fall back to
#[derive(Clone, Debug, Default, Deserialize)]
pub struct CallbackKwargs {
    #[serde(default)]
    pub standard_logging_object: Option<StandardLoggingPayload>,
    #[serde(default)]
    pub litellm_params: Option<LitellmParams>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct LitellmParams {
    #[serde(default)]
    pub metadata: Option<IndexMap<String, Json>>,
    #[serde(default)]
    pub proxy_server_request: Option<ProxyServerRequest>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct ProxyServerRequest {
    #[serde(default)]
    pub headers: Option<IndexMap<String, String>>,
}
