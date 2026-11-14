use betterlite_core::agents::json::AgentJson;
use betterlite_core::agents::provider::a2a::{
    LiteLLMSendMessageResponse, normalize_a2a_jsonrpc_response,
};

use crate::client::records::compat::SendMessageRequest;
use crate::client::records::response::SendMessageResponse;
use crate::logging_context::json_rpc_id_text;

use super::error::SendCallError;

// LiteLLM's from_a2a_response: the SDK response's null fields are dropped before the JSON-RPC id is backfilled
pub(super) fn response_from_a2a(
    response: &SendMessageResponse,
    request: &SendMessageRequest,
) -> Result<LiteLLMSendMessageResponse, SendCallError> {
    let encoded = serde_json::to_value(response).map_err(|source| SendCallError::Json {
        value: "response",
        source,
    })?;
    let request_id = AgentJson::String(json_rpc_id_text(&request.id));
    LiteLLMSendMessageResponse::from_a2a_response(&encoded, Some(&request_id))
        .map_err(SendCallError::Response)
}

// LiteLLM's from_dict: the bridge answer is normalized as sent, without the SDK dump's null filtering
pub(super) fn response_from_dict(
    response: &AgentJson,
    request_id: &str,
) -> Result<LiteLLMSendMessageResponse, SendCallError> {
    let request_id_json = AgentJson::String(request_id.to_owned());
    let normalized = normalize_a2a_jsonrpc_response(response, Some(&request_id_json))
        .map_err(SendCallError::Response)?;
    serde_json::from_value(normalized).map_err(|source| SendCallError::Json {
        value: "bridge response",
        source,
    })
}
