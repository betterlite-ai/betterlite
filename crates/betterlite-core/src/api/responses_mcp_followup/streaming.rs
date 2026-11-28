use std::collections::BTreeMap;

use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_openai::response_input::{ResponsesInput, ResponsesTool};
use thiserror::Error;

use crate::api::responses_mcp_events::event::StreamEvent;
use crate::api::responses_provider_selection::ParameterMap;
use crate::api::responses_tool_namespaces::values::Fields;

use super::params::{RequestParamsInput, build_request_params};
use super::records::{McpStreamingPlan, MetadataAuth, StreamingAuth};

#[derive(Debug, Error, PartialEq, Eq)]
pub enum StreamingError {
    #[error("litellm_metadata must be a mapping to read user_api_key_auth")]
    MetadataNotMapping,
}

pub struct StreamingRequest<'request> {
    pub input: &'request ResponsesInput,
    pub model: &'request str,
    pub tools: Option<&'request [ResponsesTool]>,
    pub previous_response_id: Option<&'request str>,
    pub call_params: &'request ParameterMap,
    pub kwargs: &'request ParameterMap,
    pub mcp_tools_with_litellm_proxy: &'request [Fields],
    pub mcp_discovery_events: &'request [StreamEvent],
    pub tool_server_map: &'request BTreeMap<String, String>,
    pub auth: &'request StreamingAuth,
}

pub fn create_mcp_streaming_response(
    request: &StreamingRequest<'_>,
) -> Result<McpStreamingPlan, StreamingError> {
    let request_params = build_request_params(&RequestParamsInput {
        input: request.input,
        model: request.model,
        tools: request.tools,
        previous_response_id: request.previous_response_id,
        call_params: request.call_params,
        kwargs: request.kwargs,
    });
    let auth = select_auth(request.auth)?;
    Ok(McpStreamingPlan {
        request_params,
        mcp_events: request.mcp_discovery_events.to_vec(),
        tool_server_map: request.tool_server_map.clone(),
        mcp_tools_with_litellm_proxy: request.mcp_tools_with_litellm_proxy.to_vec(),
        auth,
    })
}

// a present direct auth short-circuits, an absent metadata mapping reads as {}, and a present non-mapping value reaches its .get failure
fn select_auth(auth: &StreamingAuth) -> Result<Option<UserAPIKeyAuth>, StreamingError> {
    if let Some(direct) = &auth.direct {
        return Ok(Some(direct.clone()));
    }
    match &auth.metadata {
        MetadataAuth::Absent => Ok(None),
        MetadataAuth::Mapping(found) => Ok(found.clone()),
        MetadataAuth::NotMapping => Err(StreamingError::MetadataNotMapping),
    }
}
