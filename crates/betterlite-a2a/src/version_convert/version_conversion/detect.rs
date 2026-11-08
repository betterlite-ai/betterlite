use betterlite_core::agents::json::AgentJson;

use crate::agent_card::ProtocolVersion;
use crate::agent_card::merge::normalize_protocol_version;
use crate::version_convert::JsonObject;

pub(super) const SEND_ENVELOPE_KEYS: [&str; 2] = ["message", "task"];
pub(super) const STREAM_ENVELOPE_KEYS: [&str; 4] =
    ["message", "task", "statusUpdate", "artifactUpdate"];

pub(super) fn result_version(
    result: &AgentJson,
    envelope_keys: &[&str],
) -> Option<ProtocolVersion> {
    if result.get("kind").is_some() {
        return Some(ProtocolVersion::V0_3);
    }
    if envelope_keys.iter().any(|key| result.get(key).is_some()) {
        return Some(ProtocolVersion::V1_0);
    }
    None
}

pub(super) fn task_version(result: &AgentJson) -> ProtocolVersion {
    if result.get("kind").is_some() {
        return ProtocolVersion::V0_3;
    }
    ProtocolVersion::V1_0
}

pub(super) fn list_tasks_version(result: &AgentJson) -> Option<ProtocolVersion> {
    let tasks = result.get("tasks")?.as_array()?;
    let first = tasks.first()?.as_object()?;
    Some(if first.contains_key("kind") {
        ProtocolVersion::V0_3
    } else {
        ProtocolVersion::V1_0
    })
}

pub(super) fn card_version(card: &JsonObject) -> ProtocolVersion {
    if let Some(version) = card
        .get("protocolVersion")
        .and_then(normalize_protocol_version)
    {
        return version;
    }
    if card.contains_key("supportedInterfaces") {
        return ProtocolVersion::V1_0;
    }
    ProtocolVersion::V0_3
}
