use betterlite_logging::python_json::dumps;
use betterlite_models::decisions::Json;
use serde::Serialize;

use super::{ToolCall, decode_converse_json_tool};

pub const RESPONSE_FORMAT_TOOL_NAME: &str = "json_tool_call";

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct FilteredJsonTools {
    pub tools: Option<Vec<ToolCall>>,
    pub content: Option<String>,
}

#[must_use]
pub fn unwrap_bedrock_properties(json: &str) -> String {
    let Ok(Json::Object(mut fields)) = decode_converse_json_tool(json.as_bytes()) else {
        return json.to_owned();
    };
    if fields.len() != 1 {
        return json.to_owned();
    }
    fields
        .shift_remove("properties")
        .map_or_else(|| json.to_owned(), |value| dumps(&value))
}

#[must_use]
pub fn filter_json_mode_tools(
    json_mode: bool,
    tools: &[ToolCall],
    content: Option<&str>,
) -> FilteredJsonTools {
    let synthetic = synthetic_indices(tools);
    if !json_mode || synthetic.is_empty() {
        return unchanged_tools(tools, content);
    }
    if synthetic.len() == tools.len() {
        tracing::debug!("Processing JSON tool call response for response_format");
        let arguments = tools
            .first()
            .and_then(|tool| tool.function.arguments.as_deref());
        return FilteredJsonTools {
            tools: None,
            content: arguments.map_or_else(
                || content.map(str::to_owned),
                |raw| Some(unwrap_bedrock_properties(raw)),
            ),
        };
    }
    synthetic_among_real(tools, &synthetic, content)
}

fn synthetic_indices(tools: &[ToolCall]) -> Vec<usize> {
    let mut indices = Vec::new();
    for (index, tool) in tools.iter().enumerate() {
        if tool.function.name.as_deref() == Some(RESPONSE_FORMAT_TOOL_NAME) {
            indices.push(index);
        }
    }
    indices
}

fn unchanged_tools(tools: &[ToolCall], content: Option<&str>) -> FilteredJsonTools {
    FilteredJsonTools {
        tools: (!tools.is_empty()).then(|| tools.to_vec()),
        content: content.map(str::to_owned),
    }
}

fn synthetic_among_real(
    tools: &[ToolCall],
    synthetic: &[usize],
    content: Option<&str>,
) -> FilteredJsonTools {
    let first_arguments = synthetic
        .first()
        .and_then(|index| tools.get(*index))
        .and_then(|tool| tool.function.arguments.as_deref());
    let replaced = first_arguments.map_or_else(
        || content.map(str::to_owned),
        |arguments| {
            let existing = content.unwrap_or_default();
            Some(format!(
                "{existing}{}",
                unwrap_bedrock_properties(arguments)
            ))
        },
    );
    let real = real_tools(tools, synthetic);
    FilteredJsonTools {
        tools: (!real.is_empty()).then_some(real),
        content: replaced,
    }
}

fn real_tools(tools: &[ToolCall], synthetic: &[usize]) -> Vec<ToolCall> {
    tools
        .iter()
        .enumerate()
        .filter(|(index, _)| !synthetic.contains(index))
        .map(|(_, tool)| tool.clone())
        .collect()
}
