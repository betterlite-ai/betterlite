use std::collections::BTreeMap;

use betterlite_core::agents::json::AgentJson;

use super::error::SendCallError;

// LiteLLM's "if custom_llm_provider:" gate: an absent, null or empty provider takes the native path
pub(super) fn completion_provider(
    litellm_params: &BTreeMap<String, AgentJson>,
) -> Result<Option<&str>, SendCallError> {
    let Some(value) = litellm_params.get("custom_llm_provider") else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    let Some(provider) = value.as_str() else {
        return Err(SendCallError::NotText {
            field: "custom_llm_provider",
        });
    };
    Ok((!provider.is_empty()).then_some(provider))
}
