use betterlite_logging::python_json::dumps;
use betterlite_models::decisions::Json;
use betterlite_prompts::reasoning_fields::content_split::parse_content_for_reasoning;
use indexmap::IndexMap;
use serde::Serialize;

use super::{ContentBlock, ConverseContentError, ToolCall, ToolCallFunction};
use crate::model_routes::get_bedrock_tool_name;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Translation {
    pub content: String,
    pub tools: Vec<ToolCall>,
    pub reasoning_content_blocks: Option<Vec<Json>>,
    pub citations_content_blocks: Option<Vec<Json>>,
}

pub fn translate_message_content(
    content_blocks: &[ContentBlock],
    mappings: &IndexMap<String, String>,
) -> Result<Translation, ConverseContentError> {
    let mut content = String::new();
    let mut tools = Vec::new();
    let mut reasoning_blocks: Option<Vec<Json>> = None;
    let mut citation_blocks: Option<Vec<Json>> = None;
    for (index, block) in content_blocks.iter().enumerate() {
        let mut extracted_reasoning = None;
        if let Some(text) = block.text.as_deref() {
            let (reasoning, parsed_content) = parse_content_for_reasoning(Some(text));
            extracted_reasoning = reasoning;
            if let Some(visible) = parsed_content {
                content.push_str(&visible);
            }
        }
        if let Some(tool_use) = &block.tool_use {
            tools.push(tool_call_from_use(tool_use, index, mappings)?);
        }
        if let Some(reasoning) = extracted_reasoning {
            reasoning_blocks
                .get_or_insert_default()
                .push(extracted_reasoning_block(reasoning));
        }
        if let Some(raw) = &block.reasoning_content {
            reasoning_blocks.get_or_insert_default().push(raw.clone());
        }
        if let Some(raw) = &block.citations_content {
            citation_blocks.get_or_insert_default().push(raw.clone());
        }
    }
    Ok(Translation {
        content,
        tools,
        reasoning_content_blocks: reasoning_blocks,
        citations_content_blocks: citation_blocks,
    })
}

fn extracted_reasoning_block(text: String) -> Json {
    let mut reasoning = IndexMap::new();
    reasoning.insert("text".to_owned(), Json::Text(text));
    let mut block = IndexMap::new();
    block.insert("reasoningText".to_owned(), Json::Object(reasoning));
    Json::Object(block)
}

fn tool_call_from_use(
    value: &Json,
    index: usize,
    mappings: &IndexMap<String, String>,
) -> Result<ToolCall, ConverseContentError> {
    let Json::Object(fields) = value else {
        return Err(ConverseContentError::ToolUseObject);
    };
    let Some(Json::Text(name)) = fields.get("name") else {
        return Err(ConverseContentError::ToolUseField { field: "name" });
    };
    let Some(Json::Text(tool_use_id)) = fields.get("toolUseId") else {
        return Err(ConverseContentError::ToolUseField { field: "toolUseId" });
    };
    let Some(input) = fields.get("input") else {
        return Err(ConverseContentError::ToolUseField { field: "input" });
    };
    Ok(ToolCall {
        id: Some(tool_use_id.clone()),
        kind: "function".to_owned(),
        function: ToolCallFunction {
            name: Some(get_bedrock_tool_name(mappings, name).to_owned()),
            arguments: Some(dumps(input)),
        },
        index,
    })
}
