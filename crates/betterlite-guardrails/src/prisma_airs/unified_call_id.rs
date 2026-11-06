use uuid::Uuid;

use super::request_data::{LoggingObject, RequestData};
use crate::panw_prisma_airs::audit::Direction;

// Mirrors the litellm_call_id resolution in LiteLLM's fn apply_guardrail.
pub fn resolve_call_id(
    request_data: &mut RequestData,
    logging_obj: Option<&LoggingObject>,
    input_type: Direction,
    new_id: fn() -> Uuid,
) -> String {
    if let Some(call_id) = request_data
        .litellm_call_id
        .clone()
        .filter(|call_id| !call_id.is_empty())
    {
        return call_id;
    }
    if let Some(call_id) = logging_obj
        .and_then(|logging| logging.litellm_call_id.as_deref())
        .filter(|call_id| !call_id.is_empty())
    {
        return call_id.to_owned();
    }
    let call_id = match request_data
        .resolved_tool_name()
        .filter(|tool_name| !tool_name.trim().is_empty())
    {
        Some(tool_name) if input_type == Direction::Request && logging_obj.is_none() => {
            let call_id = format!("{}-{}", mcp_slug(tool_name), new_id());
            tracing::debug!(
                "PANW Prisma AIRS: synthesized MCP tr_id={call_id} for tool={tool_name}"
            );
            call_id
        }
        _ => {
            let call_id = new_id().to_string();
            tracing::warn!(
                "PANW Prisma AIRS: litellm_call_id missing, synthesized {call_id} (input_type={input_type:?})"
            );
            call_id
        }
    };
    request_data.litellm_call_id = Some(call_id.clone());
    call_id
}

#[must_use]
pub fn mcp_slug(tool_name: &str) -> String {
    let mut slug = String::new();
    for lower in tool_name.chars().flat_map(char::to_lowercase) {
        let next = if lower.is_ascii_alphanumeric() {
            Some(lower.to_ascii_lowercase())
        } else if !slug.is_empty() && !slug.ends_with('-') {
            Some('-')
        } else {
            None
        };
        if let Some(character) = next {
            slug.push(character);
        }
    }
    let trimmed_len = slug.trim_end_matches('-').len();
    slug.truncate(trimmed_len);
    let truncated: String = slug.chars().take(40).collect();
    if truncated.is_empty() {
        "mcp-tool".to_owned()
    } else {
        truncated
    }
}

mod tests;
