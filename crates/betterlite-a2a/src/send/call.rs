use std::collections::BTreeMap;

use betterlite_core::agents::json::AgentJson;
use betterlite_models::decisions::Json;

use crate::client::A2AClient;
use crate::client::records::compat::SendMessageRequest;

pub struct SendCall {
    pub a2a_client: Option<A2AClient>,
    pub request: SendMessageRequest,
    pub api_base: Option<String>,
    pub litellm_params: BTreeMap<String, AgentJson>,
    pub agent_id: Option<String>,
    pub agent_extra_headers: Option<BTreeMap<String, String>>,
}

pub struct StreamCall {
    pub a2a_client: Option<A2AClient>,
    pub request: SendMessageRequest,
    pub api_base: Option<String>,
    pub litellm_params: BTreeMap<String, AgentJson>,
    pub agent_id: Option<String>,
    pub metadata: Option<Json>,
    pub proxy_server_request: Option<Json>,
    pub agent_extra_headers: Option<BTreeMap<String, String>>,
}
