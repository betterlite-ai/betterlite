use std::collections::BTreeSet;

use betterlite_config::sections::cisco_verdict_contracts::InspectionType;
use serde::Deserialize;
use serde::de::value::{Error as ValueError, StrDeserializer};

const MCP_CALL_TYPES: [&str; 2] = ["mcp_call", "call_mcp_tool"];

#[must_use]
pub fn is_mcp_call_type(call_type: Option<&str>) -> bool {
    let Some(value) = call_type else {
        return false;
    };
    MCP_CALL_TYPES.contains(&value)
}

#[must_use]
pub const fn surface_matches(inspection_type: InspectionType, is_mcp_traffic: bool) -> bool {
    match inspection_type {
        InspectionType::Mcp => is_mcp_traffic,
        InspectionType::Chat => !is_mcp_traffic,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventHook {
    PreCall,
    DuringCall,
    PostCall,
    LoggingOnly,
    PreMcpCall,
    DuringMcpCall,
}

pub const SUPPORTED_EVENT_HOOKS: [EventHook; 6] = [
    EventHook::PreCall,
    EventHook::DuringCall,
    EventHook::PostCall,
    EventHook::LoggingOnly,
    EventHook::PreMcpCall,
    EventHook::DuringMcpCall,
];

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum EventHookInput {
    Single(String),
    Many(Vec<String>),
}

#[must_use]
pub fn normalize_event_hooks(event_hook: Option<&EventHookInput>) -> BTreeSet<EventHook> {
    let candidates: Vec<&str> = match event_hook {
        None => Vec::new(),
        Some(EventHookInput::Single(value)) => vec![value.as_str()],
        Some(EventHookInput::Many(values)) => values.iter().map(String::as_str).collect(),
    };
    candidates
        .into_iter()
        .filter_map(|candidate| {
            let deserializer = StrDeserializer::<ValueError>::new(candidate);
            EventHook::deserialize(deserializer).ok()
        })
        .collect()
}

#[must_use]
pub fn infer_inspection_from_mode(
    configured: &BTreeSet<EventHook>,
    current: InspectionType,
) -> InspectionType {
    if configured.is_empty() {
        return current;
    }
    let has_mcp = configured
        .iter()
        .any(|hook| matches!(hook, EventHook::PreMcpCall | EventHook::DuringMcpCall));
    let has_chat = configured.iter().any(|hook| {
        matches!(
            hook,
            EventHook::PreCall | EventHook::DuringCall | EventHook::PostCall
        )
    });
    match (has_mcp, has_chat) {
        (true, false) => InspectionType::Mcp,
        (false, true) => InspectionType::Chat,
        (true, true) | (false, false) => current,
    }
}

pub fn warn_if_mode_surface_mismatch(
    guardrail_name: &str,
    inspection_type: InspectionType,
    configured: &BTreeSet<EventHook>,
) {
    let mcp_hooks: BTreeSet<EventHook> = configured
        .iter()
        .copied()
        .filter(|hook| matches!(hook, EventHook::PreMcpCall | EventHook::DuringMcpCall))
        .collect();
    let chat_hooks: BTreeSet<EventHook> = configured
        .iter()
        .copied()
        .filter(|hook| {
            matches!(
                hook,
                EventHook::PreCall | EventHook::DuringCall | EventHook::PostCall
            )
        })
        .collect();
    if mcp_hooks.is_empty() || chat_hooks.is_empty() {
        return;
    }
    let unused_hooks = if inspection_type == InspectionType::Chat {
        &mcp_hooks
    } else {
        &chat_hooks
    };
    tracing::warn!(
        guardrail = guardrail_name,
        inspection_type = ?inspection_type,
        configured = ?configured,
        unused_hooks = ?unused_hooks,
        "Cisco AI Defense guardrail has mixed event hooks; some hooks will not fire for this surface"
    );
}

mod tests;
