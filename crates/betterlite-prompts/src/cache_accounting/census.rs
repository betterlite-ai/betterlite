use crate::arguments::json::Json;
use crate::cache_accounting::carriers::{carries_cache_breakpoint, tool_carries_cache_breakpoint};
use crate::cache_accounting::constants::MAX_CACHE_CONTROL_BLOCKS;
use crate::cache_accounting::json_views::object_list;
use crate::cache_accounting::points::CacheControlInjectionPoint;

// A message counts its own mark and one per marked content block; anything that is not an object counts nothing.
#[must_use]
pub fn count_cache_control_blocks(message: &Json) -> usize {
    let Some(fields) = message.as_object() else {
        return 0;
    };
    let top = usize::from(carries_cache_breakpoint(message));
    let blocks = match fields.get("content") {
        Some(Json::List(items)) => items
            .iter()
            .filter(|block| carries_cache_breakpoint(block))
            .count(),
        Some(_) | None => 0,
    };
    top.saturating_add(blocks)
}

#[must_use]
pub fn count_request_breakpoints(messages: &[Json], system: Option<&Json>) -> usize {
    let system_blocks = match system {
        Some(Json::List(items)) => items
            .iter()
            .filter(|block| carries_cache_breakpoint(block))
            .count(),
        Some(_) | None => 0,
    };
    messages
        .iter()
        .map(count_cache_control_blocks)
        .sum::<usize>()
        .saturating_add(system_blocks)
}

// A top-level cache_control is Anthropic's automatic caching: one more breakpoint the provider cap counts.
#[must_use]
pub fn count_external_breakpoints(
    tools: Option<&[Json]>,
    cache_control: Option<&Json>,
    request_kwargs: Option<&Json>,
) -> usize {
    let extra_body = request_kwargs
        .and_then(Json::as_object)
        .and_then(|kwargs| kwargs.get("extra_body"))
        .and_then(Json::as_object);
    let wire_cache_control = extra_body
        .and_then(|body| body.get("cache_control"))
        .map_or_else(|| cache_control.and_then(present), present);
    let wire_tools = extra_body
        .and_then(|body| body.get("tools"))
        .map_or(tools, object_list);
    let tool_blocks = wire_tools
        .unwrap_or_default()
        .iter()
        .filter(|tool| tool_carries_cache_breakpoint(tool))
        .count();
    let envelope_messages = extra_body
        .and_then(|body| body.get("messages"))
        .and_then(object_list)
        .unwrap_or_default();
    let envelope_system = extra_body.and_then(|body| body.get("system"));
    let envelope_blocks = count_request_breakpoints(envelope_messages, envelope_system);
    usize::from(wire_cache_control.is_some())
        .saturating_add(tool_blocks)
        .saturating_add(envelope_blocks)
}

const fn present(value: &Json) -> Option<&Json> {
    match value {
        Json::Null => None,
        Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) | Json::Object(_) => {
            Some(value)
        }
    }
}

// The native messages transforms drop the extra_body envelope while the chat bridge merges it, so the cap reserves for the larger census.
#[must_use]
pub fn count_route_breakpoints(
    tools: Option<&[Json]>,
    cache_control: Option<&Json>,
    request_kwargs: Option<&Json>,
) -> usize {
    let direct = count_external_breakpoints(tools, cache_control, None);
    let envelope = count_external_breakpoints(tools, cache_control, request_kwargs);
    direct.max(envelope)
}

// OpenAI's cap counts only its own block markers, so no slot is reserved there.
#[must_use]
pub fn reserved_outside_messages(
    points: &[CacheControlInjectionPoint],
    external_breakpoints: usize,
    openai_dialect: bool,
) -> usize {
    if openai_dialect {
        return 0;
    }
    let tool_config = points
        .iter()
        .any(|point| matches!(point, CacheControlInjectionPoint::ToolConfig(_)));
    external_breakpoints.saturating_add(usize::from(tool_config))
}

// Anthropic and Bedrock reject a request past four cache_control blocks, and a tool_config point becomes a cachePoint the converse transform never counts, so it rides only while a slot is left.
#[must_use]
pub fn points_with_a_slot_left(
    points: &[CacheControlInjectionPoint],
    breakpoints_on_wire: usize,
    openai_dialect: bool,
) -> Vec<CacheControlInjectionPoint> {
    if openai_dialect || breakpoints_on_wire < MAX_CACHE_CONTROL_BLOCKS {
        return points.to_vec();
    }
    points
        .iter()
        .filter(|point| !matches!(point, CacheControlInjectionPoint::ToolConfig(_)))
        .cloned()
        .collect()
}

pub struct RequestCacheMarks<'input> {
    pub messages: &'input [Json],
    pub system: Option<&'input Json>,
    pub tools: Option<&'input [Json]>,
    pub cache_control: Option<&'input Json>,
    pub request_kwargs: Option<&'input Json>,
}

// Only the automatic defaults stand down on a client mark; configured injection points apply alongside it.
#[must_use]
pub fn request_has_cache_control(marks: &RequestCacheMarks<'_>, on_messages_route: bool) -> bool {
    let external = if on_messages_route {
        count_route_breakpoints(marks.tools, marks.cache_control, marks.request_kwargs)
    } else {
        count_external_breakpoints(marks.tools, marks.cache_control, marks.request_kwargs)
    };
    count_request_breakpoints(marks.messages, marks.system).saturating_add(external) > 0
}
