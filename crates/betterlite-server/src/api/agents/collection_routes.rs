mod body;
pub mod guards;
mod handlers;
pub mod merge;
pub mod processing;

mod routes;
mod security;
mod tests;

use std::sync::Arc;

use axum::Router;
use axum::routing::{get, post};
use betterlite_config::sections::request_body::Settings;
use betterlite_core::api::api_boundaries::ApiLimits;

use crate::pre_call::key::PreCallKey;
use crate::pre_call_step::PreCall;

use processing::AgentRequestProcessing;

pub const AGENTS_PATH: &str = "/v1beta/agents";
pub const AGENT_PATH: &str = "/v1beta/agents/{name}";
pub const AGENT_VERSIONS_PATH: &str = "/v1beta/agents/{name}/versions";

// the proxy_server globals LiteLLM re-imports per request live here once, in the wiring that builds this state
pub struct AgentCollectionState<K: PreCallKey> {
    pub pre_call: Arc<dyn PreCall<K>>,
    pub processing: Arc<dyn AgentRequestProcessing<K>>,
    pub request_body: Settings,
    pub limits: ApiLimits,
    pub version: String,
}

impl<K: PreCallKey> Clone for AgentCollectionState<K> {
    fn clone(&self) -> Self {
        Self {
            pre_call: Arc::clone(&self.pre_call),
            processing: Arc::clone(&self.processing),
            request_body: self.request_body,
            limits: self.limits,
            version: self.version.clone(),
        }
    }
}

pub fn routes<K>(state: AgentCollectionState<K>) -> Router
where
    K: PreCallKey + Clone + Send + Sync + 'static,
{
    Router::new()
        .route(
            AGENTS_PATH,
            post(handlers::create_agent::<K>).get(handlers::list_agents::<K>),
        )
        .route(
            AGENT_PATH,
            get(handlers::get_agent::<K>).delete(handlers::delete_agent::<K>),
        )
        .route(AGENT_VERSIONS_PATH, get(handlers::list_agent_versions::<K>))
        .with_state(state)
}
