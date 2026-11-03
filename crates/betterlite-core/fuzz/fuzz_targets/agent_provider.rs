#![no_main]

use betterlite_core::agents::json::AgentJson;
use betterlite_core::agents::provider::a2a::{
    LiteLLMSendMessageResponse, normalize_a2a_jsonrpc_response,
};
use betterlite_core::agents::provider::{
    AgentCreateResponse, AgentDeleteResult, AgentListResponse, AgentProviderError,
    AgentVersionsResponse, ListAgentsResponse,
};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    drop(serde_json::from_slice::<AgentCreateResponse>(data));
    drop(serde_json::from_slice::<AgentDeleteResult>(data));
    drop(serde_json::from_slice::<AgentListResponse>(data));
    drop(serde_json::from_slice::<AgentVersionsResponse>(data));
    drop(serde_json::from_slice::<ListAgentsResponse>(data));
    drop(serde_json::from_slice::<LiteLLMSendMessageResponse>(data));
    if let Ok(payload) = serde_json::from_slice::<AgentJson>(data) {
        drop(
            normalize_a2a_jsonrpc_response(&payload, Some(&payload)).and_then(|normalized| {
                serde_json::from_value::<LiteLLMSendMessageResponse>(normalized)
                    .map_err(|source| AgentProviderError::Decode { source })
            }),
        );
        drop(LiteLLMSendMessageResponse::from_a2a_response(
            &payload, None,
        ));
    }
});
