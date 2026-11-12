use betterlite_context::provider_message_primitives::FunctionKind;
use betterlite_models::decisions::Json;
use serde::Serialize;

use super::error::StreamStateError;
use super::state::StreamState;
use super::wire::{
    self, ContentBlockDelta, ContentBlockStartBody, ContentBlockStartPayload, ContentDelta,
    JsonFields,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DeltaProjection {
    pub text: String,
    pub tool_use: Option<ToolCallChunk>,
    pub thinking_blocks: Vec<ThinkingBlock>,
    pub provider_specific_fields: JsonFields,
    pub reasoning_content: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ToolCallChunk {
    pub id: Option<String>,
    #[serde(rename = "type")]
    pub kind: FunctionKind,
    pub function: ToolFunctionChunk,
    pub index: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ToolFunctionChunk {
    pub name: Option<String>,
    pub arguments: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ThinkingBlock {
    Thinking {
        #[serde(skip_serializing_if = "Option::is_none")]
        signature: Option<String>,
        thinking: String,
    },
    RedactedThinking {
        data: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ContentBlockStartShape {
    ToolUse,
    Text,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ContentBlockStart {
    pub shape: ContentBlockStartShape,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub index: Option<i64>,
    pub content_block: ContentBlockStartBody,
}

enum DeltaBranch<'delta> {
    PartialJson(&'delta str),
    Citation(&'delta Json),
    Thinking,
    Compaction(&'delta Json),
    None,
}

pub fn get_content_block_start(chunk: &Json) -> Result<ContentBlockStart, StreamStateError> {
    let payload = wire::decode::<ContentBlockStartPayload>(chunk)
        .map_err(|source| StreamStateError::Decode { source })?;
    let shape = if payload.content_block.kind == "tool_use" {
        ContentBlockStartShape::ToolUse
    } else {
        ContentBlockStartShape::Text
    };
    Ok(ContentBlockStart {
        shape,
        index: payload.index,
        content_block: payload.content_block,
    })
}

pub fn redacted_thinking_content(
    start: &ContentBlockStart,
    provider_specific_fields: &mut JsonFields,
) -> Result<Vec<ThinkingBlock>, StreamStateError> {
    let data = start
        .content_block
        .data
        .clone()
        .ok_or(StreamStateError::FieldType { field: "data" })?;
    let thinking_blocks = vec![ThinkingBlock::RedactedThinking { data }];
    set_thinking_field(provider_specific_fields, &thinking_blocks);
    Ok(thinking_blocks)
}

pub fn content_block_delta_helper(
    state: &mut StreamState,
    chunk: &Json,
) -> Result<DeltaProjection, StreamStateError> {
    let content_block = wire::decode::<ContentBlockDelta>(chunk)
        .map_err(|source| StreamStateError::Decode { source })?;
    if let Some(text) = content_block.delta.text.clone() {
        return Ok(DeltaProjection {
            text,
            tool_use: None,
            thinking_blocks: Vec::new(),
            provider_specific_fields: JsonFields::new(),
            reasoning_content: None,
        });
    }
    state.content_blocks.push(content_block.clone());
    let mut projection = DeltaProjection {
        text: String::new(),
        tool_use: None,
        thinking_blocks: Vec::new(),
        provider_specific_fields: JsonFields::new(),
        reasoning_content: None,
    };
    match delta_branch(&content_block.delta) {
        DeltaBranch::PartialJson(partial_json) => {
            projection.tool_use = tool_call_chunk(state, partial_json);
        }
        DeltaBranch::Citation(citation) => {
            projection
                .provider_specific_fields
                .insert("citation".to_owned(), citation.clone());
        }
        DeltaBranch::Thinking => apply_thinking(state, &content_block, &mut projection),
        DeltaBranch::Compaction(content) => {
            projection
                .provider_specific_fields
                .insert("compaction_delta".to_owned(), compaction_delta(content));
        }
        DeltaBranch::None => {}
    }
    Ok(projection)
}

fn delta_branch(delta: &ContentDelta) -> DeltaBranch<'_> {
    if let Some(partial_json) = delta.partial_json.as_deref() {
        return DeltaBranch::PartialJson(partial_json);
    }
    if let Some(citation) = delta.citation.as_ref() {
        return DeltaBranch::Citation(citation);
    }
    if delta.thinking.is_some() || delta.signature.is_some() {
        return DeltaBranch::Thinking;
    }
    if delta.kind == "compaction_delta"
        && let Some(content) = delta.content.as_ref()
    {
        return DeltaBranch::Compaction(content);
    }
    DeltaBranch::None
}

fn tool_call_chunk(state: &StreamState, partial_json: &str) -> Option<ToolCallChunk> {
    let in_tool_block = matches!(
        state.current_content_block_type.as_deref(),
        Some("tool_use" | "server_tool_use")
    );
    in_tool_block.then(|| ToolCallChunk {
        id: None,
        kind: FunctionKind::Function,
        function: ToolFunctionChunk {
            name: None,
            arguments: partial_json.to_owned(),
        },
        index: state.tool_index,
    })
}

fn apply_thinking(
    state: &mut StreamState,
    content_block: &ContentBlockDelta,
    projection: &mut DeltaProjection,
) {
    if let Some(Json::Text(thinking)) = &content_block.delta.thinking
        && !thinking.is_empty()
    {
        state.reasoning_content_chunks.push(thinking.clone());
        projection.reasoning_content = Some(thinking.clone());
        projection.thinking_blocks = vec![ThinkingBlock::Thinking {
            thinking: thinking.clone(),
            signature: None,
        }];
        set_thinking_field(
            &mut projection.provider_specific_fields,
            &projection.thinking_blocks,
        );
    }
    let Some(Json::Text(signature)) = &content_block.delta.signature else {
        return;
    };
    if signature.is_empty() {
        return;
    }
    let thinking = concatenated_thinking(state);
    projection.thinking_blocks = vec![ThinkingBlock::Thinking {
        thinking,
        signature: Some(signature.clone()),
    }];
    set_thinking_field(
        &mut projection.provider_specific_fields,
        &projection.thinking_blocks,
    );
    if projection.reasoning_content.is_none() {
        projection.reasoning_content = Some(String::new());
    }
}

fn concatenated_thinking(state: &StreamState) -> String {
    let mut text = String::new();
    for block in &state.content_blocks {
        if let Some(Json::Text(thinking)) = &block.delta.thinking {
            text.push_str(thinking);
        }
    }
    text
}

fn compaction_delta(content: &Json) -> Json {
    let mut fields = JsonFields::new();
    fields.insert("type".to_owned(), Json::Text("compaction_delta".to_owned()));
    fields.insert("content".to_owned(), content.clone());
    Json::Object(fields)
}

fn set_thinking_field(
    provider_specific_fields: &mut JsonFields,
    thinking_blocks: &[ThinkingBlock],
) {
    let Some(blocks) = wire::to_json(thinking_blocks) else {
        return;
    };
    provider_specific_fields.insert("thinking_blocks".to_owned(), blocks);
}
