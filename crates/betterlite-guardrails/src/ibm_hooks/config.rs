use serde::{Deserialize, Serialize};

use super::IbmGuardrail;
use super::data::PromptData;

mod tests;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EventHook {
    PreCall,
    PostCall,
    DuringCall,
    LoggingOnly,
    PreMcpCall,
    DuringMcpCall,
    PostMcpCall,
    RealtimeInputTranscription,
}

// The event-type half of LiteLLM's should_run_guardrail; the requested-guardrail and admin-metadata gates belong to the framework.
#[must_use]
pub fn should_run_guardrail(guardrail: &IbmGuardrail, event_type: EventHook) -> bool {
    let Some(event_hooks) = guardrail.event_hooks.as_deref() else {
        return true;
    };
    event_hooks.contains(&event_type)
}

// Mirrors LiteLLM's add_guardrail_to_applied_guardrails_header for the proxy metadata bucket.
pub fn record_applied_guardrail(data: &mut PromptData, guardrail_name: &str) {
    let metadata = match &mut data.litellm_metadata {
        Some(metadata) => metadata,
        None => data.metadata.get_or_insert_default(),
    };
    let applied = metadata.applied_guardrails.get_or_insert_default();
    if !applied.iter().any(|name| name == guardrail_name) {
        applied.push(guardrail_name.to_owned());
    }
}
