use betterlite_models::decisions::{Extra, Json};
use indexmap::IndexMap;

use crate::converse_content::json_mode::RESPONSE_FORMAT_TOOL_NAME;
use crate::converse_events::ConverseStreamState;
use crate::converse_events::error::ConverseEventError;
use crate::converse_events::projection::EventParts;
use crate::converse_events::response::{ToolCallChunk, ToolFunctionChunk};
use crate::converse_events::thinking::translate_thinking_blocks;
use crate::converse_events::wire;
use crate::model_routes::get_bedrock_tool_name;

pub(super) fn start_event(
    state: &mut ConverseStreamState,
    value: &Json,
    parts: &mut EventParts,
) -> Result<(), ConverseEventError> {
    let start = wire::object(value, "start")?;
    state.content_blocks.clear();
    let tool = start
        .get("toolUse")
        .filter(|candidate| !matches!(candidate, Json::Null));
    if let Some(tool_value) = tool {
        return start_tool(state, tool_value, parts);
    }
    let reasoning = start
        .get("reasoningContent")
        .filter(|candidate| !matches!(candidate, Json::Null));
    if let Some(reasoning_value) = reasoning {
        parts.thinking_blocks = Some(translate_thinking_blocks(wire::object(
            reasoning_value,
            "start.reasoningContent",
        )?)?);
        parts.provider_fields = Some(Extra::from([(
            "reasoningContent".to_owned(),
            reasoning_value.clone(),
        )]));
    }
    Ok(())
}

fn start_tool(
    state: &mut ConverseStreamState,
    value: &Json,
    parts: &mut EventParts,
) -> Result<(), ConverseEventError> {
    let tool = wire::object(value, "start.toolUse")?;
    let name = wire::required_text(tool, "name", "start.toolUse.name")?;
    let response_name = get_bedrock_tool_name(&state.mappings, name).to_owned();
    state.current_tool_name = Some(response_name.clone());
    if state.json_mode && response_name == RESPONSE_FORMAT_TOOL_NAME {
        return Ok(());
    }
    let index = match state.tool_calls_index {
        Some(current) => current
            .checked_add(1)
            .ok_or(ConverseEventError::IndexOverflow)?,
        None => 0,
    };
    state.tool_calls_index = Some(index);
    let tool_use_id = wire::required_text(tool, "toolUseId", "start.toolUse.toolUseId")?;
    parts.tool_use = Some(ToolCallChunk {
        id: Some(tool_use_id.to_owned()),
        kind: "function",
        function: ToolFunctionChunk {
            name: Some(response_name),
            arguments: String::new(),
        },
        index,
    });
    Ok(())
}

pub(super) fn delta_event(
    state: &mut ConverseStreamState,
    value: &Json,
    index: i64,
    parts: &mut EventParts,
) -> Result<(), ConverseEventError> {
    let delta = wire::object(value, "delta")?;
    if delta.contains_key("text") {
        parts.content = wire::optional_text(delta.get("text"), "delta.text")?;
        return Ok(());
    }
    if let Some(tool) = delta.get("toolUse") {
        return delta_tool(state, delta, tool, index, parts);
    }
    if let Some(reasoning) = delta.get("reasoningContent") {
        return delta_reasoning(reasoning, parts);
    }
    if let Some(citations) = delta.get("citationsContent") {
        parts.provider_fields = Some(Extra::from([(
            "citationsContent".to_owned(),
            citations.clone(),
        )]));
    }
    Ok(())
}

fn delta_tool(
    state: &mut ConverseStreamState,
    delta: &IndexMap<String, Json>,
    tool: &Json,
    index: i64,
    parts: &mut EventParts,
) -> Result<(), ConverseEventError> {
    state.content_blocks.push(Json::Object(delta.clone()));
    let tool_block = wire::object(tool, "delta.toolUse")?;
    let input = wire::required_text(tool_block, "input", "delta.toolUse.input")?;
    if state.json_mode && state.current_tool_name.as_deref() == Some(RESPONSE_FORMAT_TOOL_NAME) {
        parts.content = Some(input.to_owned());
        return Ok(());
    }
    parts.tool_use = Some(ToolCallChunk {
        id: None,
        kind: "function",
        function: ToolFunctionChunk {
            name: None,
            arguments: input.to_owned(),
        },
        index: state.tool_calls_index.unwrap_or(index),
    });
    Ok(())
}

pub(super) fn stop_event(
    state: &mut ConverseStreamState,
    index: i64,
) -> Result<Option<ToolCallChunk>, ConverseEventError> {
    if state.json_mode && state.current_tool_name.as_deref() == Some(RESPONSE_FORMAT_TOOL_NAME) {
        state.current_tool_name = None;
        return Ok(None);
    }
    state.current_tool_name = None;
    if !state.check_empty_tool_call_args()? {
        return Ok(None);
    }
    Ok(Some(ToolCallChunk {
        id: None,
        kind: "function",
        function: ToolFunctionChunk {
            name: None,
            arguments: "{}".to_owned(),
        },
        index: state.tool_calls_index.unwrap_or(index),
    }))
}

fn delta_reasoning(value: &Json, parts: &mut EventParts) -> Result<(), ConverseEventError> {
    let block = wire::object(value, "delta.reasoningContent")?;
    parts.provider_fields = Some(Extra::from([(
        "reasoningContent".to_owned(),
        value.clone(),
    )]));
    parts.reasoning_content =
        wire::optional_text(block.get("text"), "delta.reasoningContent.text")?;
    let blocks = translate_thinking_blocks(block)?;
    if !blocks.is_empty() && parts.reasoning_content.is_none() {
        parts.reasoning_content = Some(String::new());
    }
    parts.thinking_blocks = Some(blocks);
    Ok(())
}
