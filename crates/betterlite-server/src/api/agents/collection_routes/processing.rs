use async_trait::async_trait;
use axum::http::request::Parts;
use axum::response::Response;
use betterlite_logging::logging_state::LoggingState;
use betterlite_openai::openai_base_config::Object as JsonMap;

use crate::pre_call::failure::PreCallFailure;
use crate::pre_call::key::PreCallKey;
use crate::pre_call::route_type::RouteType;

// what base_process_llm_request reads per call; the proxy globals it also reads arrive with the port's construction
pub struct AgentCall<'call, K> {
    pub data: &'call JsonMap,
    pub logging: LoggingState,
    pub request: &'call Parts,
    pub key: &'call K,
    pub route_type: RouteType,
    pub version: &'call str,
}

// LiteLLM's base_process_llm_request and _handle_llm_api_exception; the route wraps its call in the same try/except
#[async_trait]
pub trait AgentRequestProcessing<K: PreCallKey>: Send + Sync {
    async fn process(&self, call: AgentCall<'_, K>) -> Result<Response, PreCallFailure>;

    async fn handle_exception(&self, failure: PreCallFailure, data: &JsonMap, key: &K) -> Response;
}
