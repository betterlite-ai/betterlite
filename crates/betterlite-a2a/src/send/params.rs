use std::collections::BTreeMap;

use betterlite_core::agents::json::AgentJson;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::client::records::compat::SendMessageRequest;
use crate::completion_bridge::wire::MessageSendParams;
use crate::logging_context::A2A_COST_PARAM_KEYS;

use super::error::SendCallError;

// LiteLLM's request.params.model_dump(mode="json"): the bridge handler reads the request as plain parameters
pub(super) fn bridge_params(
    request: &SendMessageRequest,
) -> Result<MessageSendParams, SendCallError> {
    let encoded = serde_json::to_value(&request.params).map_err(|source| SendCallError::Json {
        value: "request params",
        source,
    })?;
    serde_json::from_value(encoded).map_err(|source| SendCallError::Json {
        value: "request params",
        source,
    })
}

// Only the pricing keys LiteLLM's _a2a_cost_params selects reach the logging object
pub(super) fn pricing_params(
    litellm_params: &BTreeMap<String, AgentJson>,
) -> Result<IndexMap<String, Json>, SendCallError> {
    let mut pricing = IndexMap::new();
    for key in A2A_COST_PARAM_KEYS {
        let Some(value) = litellm_params.get(key).filter(|value| !value.is_null()) else {
            continue;
        };
        let price =
            serde_json::from_value(value.clone()).map_err(|source| SendCallError::Json {
                value: "litellm_params pricing",
                source,
            })?;
        pricing.insert(key.to_owned(), price);
    }
    Ok(pricing)
}
