use betterlite_core::api::request_destination::RequestData;

use super::request_fields::text_field;

mod tests;

// LiteLLM's _get_resolved_agent_id: the key's own agent assignment wins over request metadata
#[must_use]
pub fn resolved_agent_id<'input>(
    key_agent_id: Option<&'input str>,
    data: &'input RequestData,
) -> Option<&'input str> {
    if let Some(agent_id) = key_agent_id {
        return Some(agent_id);
    }
    data.get("metadata")
        .and_then(|bucket| text_field(bucket, "agent_id"))
}
