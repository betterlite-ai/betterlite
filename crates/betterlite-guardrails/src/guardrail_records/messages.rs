use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_context::local_ttl_cache::dual::DualLocalCache;
use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::responses_input_messages::{InputRequest, input_messages};

use super::error::GuardrailRecordError;

// LiteLLM's get_guardrails_messages_for_call_type
pub fn messages_for_call_type(
    call_type: &str,
    request_data: Option<&BTreeMap<String, GuardrailValue>>,
    tool_cache: &DualLocalCache,
) -> Result<Option<Vec<GuardrailValue>>, GuardrailRecordError> {
    let Some(data) = request_data else {
        return Ok(None);
    };
    match call_type {
        "completion" | "acompletion" | "anthropic_messages" | "call_mcp_tool"
        | "list_mcp_tools" => match data.get("messages") {
            None | Some(GuardrailValue::Null) => Ok(None),
            Some(GuardrailValue::Array(messages)) => Ok(Some(messages.clone())),
            Some(_) => Err(GuardrailRecordError::MessagesShape {
                call_type: call_type.to_owned(),
            }),
        },
        "responses" | "aresponses" => {
            if matches!(data.get("input"), None | Some(GuardrailValue::Null)) {
                return Ok(None);
            }
            let request: InputRequest = serde_json::to_value(data)
                .and_then(serde_json::from_value)
                .map_err(|source| GuardrailRecordError::Json { source })?;
            let messages = input_messages(&request, false, ApiLimits::default(), tool_cache)?;
            serde_json::to_value(messages)
                .and_then(serde_json::from_value)
                .map_err(|source| GuardrailRecordError::Json { source })
        }
        _ => Ok(None),
    }
}
