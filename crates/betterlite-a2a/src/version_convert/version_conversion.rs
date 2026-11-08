use betterlite_core::agents::json::AgentJson;
use betterlite_core::agents::provider::a2a::JsonRpcId;

use crate::agent_card::ProtocolVersion;
use crate::version_convert::JsonObject;

mod card;
mod detect;
mod error;
mod json;
mod request;
mod tests;
mod v03;
mod v1;

use error::ConvertError;

enum Direction {
    Upgrade,
    Downgrade,
}

const fn direction(current: ProtocolVersion, target: ProtocolVersion) -> Option<Direction> {
    match (current, target) {
        (ProtocolVersion::V0_3, ProtocolVersion::V1_0) => Some(Direction::Upgrade),
        (ProtocolVersion::V1_0, ProtocolVersion::V0_3) => Some(Direction::Downgrade),
        (ProtocolVersion::V0_3, ProtocolVersion::V0_3)
        | (ProtocolVersion::V1_0, ProtocolVersion::V1_0) => None,
    }
}

pub(super) fn result_for_method(
    result: &AgentJson,
    target: ProtocolVersion,
    method: &str,
) -> Option<JsonObject> {
    match method {
        "message/send" => {
            let current = detect::result_version(result, &detect::SEND_ENVELOPE_KEYS)?;
            match direction(current, target)? {
                Direction::Upgrade => attempt("send result", || v1::send_result(result)),
                Direction::Downgrade => attempt("send result", || v03::send_result(result)),
            }
        }
        "tasks/get" | "tasks/cancel" => {
            let current = detect::task_version(result);
            match direction(current, target)? {
                Direction::Upgrade => attempt("task", || v1::task_json(result)),
                Direction::Downgrade => attempt("task", || v03::task(result)),
            }
        }
        "tasks/list" => {
            let current = detect::list_tasks_version(result)?;
            match direction(current, target)? {
                Direction::Upgrade => attempt("list tasks result", || v1::list_tasks(result)),
                Direction::Downgrade => attempt("list tasks result", || v03::list_tasks(result)),
            }
        }
        _ => None,
    }
}

pub(super) fn stream_for(
    result: &AgentJson,
    target: ProtocolVersion,
    request_id: Option<&JsonRpcId>,
) -> Option<JsonObject> {
    let current = detect::result_version(result, &detect::STREAM_ENVELOPE_KEYS)?;
    match direction(current, target)? {
        Direction::Upgrade => attempt("stream event", || v1::stream_result(result)),
        Direction::Downgrade => attempt("stream event", || v03::stream_result(result, request_id)),
    }
}

pub(super) fn card_needs_conversion(card: &JsonObject, target: ProtocolVersion) -> bool {
    if detect::card_version(card) != target {
        return true;
    }
    target == ProtocolVersion::V0_3 && card.contains_key("supportedInterfaces")
}

pub(super) fn convert_card(card: &JsonObject, target: ProtocolVersion) -> Option<JsonObject> {
    match target {
        ProtocolVersion::V1_0 => attempt("agent card", || card::upgrade(card)),
        ProtocolVersion::V0_3 => attempt("agent card", || card::downgrade(card)),
    }
}

pub(super) fn request_for(request: &AgentJson, method: &str) -> Option<JsonObject> {
    attempt(&format!("request params ({method})"), || {
        request::lower(request, method)
    })
}

fn attempt<F>(label: &str, convert: F) -> Option<JsonObject>
where
    F: FnOnce() -> Result<JsonObject, ConvertError>,
{
    match convert() {
        Ok(converted) => Some(converted),
        Err(source) => {
            tracing::debug!(
                error = %source,
                label,
                "A2A version conversion failed; keeping the original payload"
            );
            None
        }
    }
}
