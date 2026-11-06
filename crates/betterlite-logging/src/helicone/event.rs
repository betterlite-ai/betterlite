use betterlite_models::decisions::Json;
use chrono::{DateTime, Utc};
use indexmap::IndexMap;

#[derive(Debug, Clone)]
pub struct SuccessEvent {
    pub model: String,
    pub messages: Vec<Json>,
    pub response: Json,
    pub start_time: DateTime<Utc>,
    pub end_time: DateTime<Utc>,
    pub litellm_params: LitellmParams,
}

#[derive(Debug, Clone, Default)]
pub struct LitellmParams {
    pub custom_llm_provider: String,
    pub metadata: IndexMap<String, Json>,
    pub proxy_server_request: Option<ProxyServerRequest>,
}

#[derive(Debug, Clone, Default)]
pub struct ProxyServerRequest {
    pub headers: IndexMap<String, String>,
}
