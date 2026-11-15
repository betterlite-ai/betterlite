use betterlite_anthropic::message_replay::is_empty_unsigned_thinking;
use betterlite_anthropic::message_replay::tool_ids::normalize_tool_use_id;
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::arguments::parse::parse_tool_call_arguments;
use indexmap::IndexMap;

use crate::api::messages_chat_output::error::{ARGUMENT_CONTEXT, ChatOutputError};
use crate::api::messages_chat_output::output::{
    AnthropicStopReason, ContentBlock, ContentEntry, SignatureFields,
};
use crate::api::messages_chat_output::records::{
    ChoiceRecord, MessageRecord, ToolCallRecord, refusal_text,
};
use crate::api::messages_content_primitives::signatures::{
    FunctionFields, ToolCallFields, compaction_blocks, tool_call_signature,
};

// "stop" and every unknown reason share end_turn, and a dedicated "stop" arm would only add a dead mutant.
#[must_use]
pub const fn anthropic_finish_reason(reason: &str) -> AnthropicStopReason {
    match reason {
        "length" => AnthropicStopReason::MaxTokens,
        "tool_calls" => AnthropicStopReason::ToolUse,
        "content_filter" | "refusal" => AnthropicStopReason::Refusal,
        _ => AnthropicStopReason::EndTurn,
    }
}

#[must_use]
pub fn content_blocks(
    choices: &[ChoiceRecord],
    tool_name_mapping: Option<&IndexMap<String, String>>,
) -> Result<Vec<ContentEntry>, ChatOutputError> {
    let mut entries = Vec::new();
    for choice in choices {
        choice_entries(&choice.message, tool_name_mapping, &mut entries)?;
    }
    Ok(entries)
}

fn choice_entries(
    message: &MessageRecord,
    tool_name_mapping: Option<&IndexMap<String, String>>,
    entries: &mut Vec<ContentEntry>,
) -> Result<(), ChatOutputError> {
    let compaction = compaction_blocks(&message.provider_specific_fields);
    // LiteLLM suppresses an empty text block only beside this choice's own compaction blocks.
    let has_compaction = !compaction.is_empty();
    entries.extend(
        compaction
            .into_iter()
            .map(|block| ContentEntry::Compaction(block.clone())),
    );
    entries.extend(thinking_entries(message)?);
    if let Some(content) = &message.content
        && (!content.is_empty() || !has_compaction)
    {
        entries.push(ContentEntry::Block(ContentBlock::Text {
            text: content.clone(),
        }));
    }
    if let Some(refusal) = refusal_text(&message.refusal, &message.provider_specific_fields) {
        entries.push(ContentEntry::Block(ContentBlock::Text { text: refusal }));
    }
    for tool_call in &message.tool_calls {
        entries.push(tool_use_entry(tool_call, tool_name_mapping)?);
    }
    Ok(())
}

fn thinking_entries(message: &MessageRecord) -> Result<Vec<ContentEntry>, ChatOutputError> {
    let blocks = message
        .thinking_blocks
        .as_ref()
        .filter(|blocks| !blocks.is_empty());
    let Some(blocks) = blocks else {
        if message.reasoning_content.is_empty() {
            return Ok(Vec::new());
        }
        return Ok(vec![ContentEntry::Block(ContentBlock::Thinking {
            thinking: message.reasoning_content.clone(),
            signature: None,
        })]);
    };
    let mut entries = Vec::new();
    for block in blocks {
        match block.get("type").and_then(Json::as_str) {
            Some("thinking") => {
                entries.extend(response_thinking_entry(block)?);
            }
            Some("redacted_thinking") => {
                let data = response_text(block, "data")?.unwrap_or_default();
                entries.push(ContentEntry::Block(ContentBlock::RedactedThinking { data }));
            }
            Some(_) | None => {}
        }
    }
    Ok(entries)
}

// A signed empty thinking block survives; an empty block without a signature is dropped.
fn response_thinking_entry(block: &Object) -> Result<Option<ContentEntry>, ChatOutputError> {
    if is_empty_unsigned_thinking(&Json::Object(block.clone())) {
        return Ok(None);
    }
    Ok(Some(ContentEntry::Block(ContentBlock::Thinking {
        thinking: response_text(block, "thinking")?.unwrap_or_default(),
        signature: response_text(block, "signature")?,
    })))
}

// str(value) if value is not None else None, with LiteLLM's "" default for an absent key.
fn response_text(block: &Object, key: &'static str) -> Result<Option<String>, ChatOutputError> {
    let Some(value) = block.get(key) else {
        return Ok(Some(String::new()));
    };
    match value {
        Json::Text(text) => Ok(Some(text.clone())),
        Json::Null => Ok(None),
        Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
            Err(ChatOutputError::NonTextThinkingField { field: key })
        }
    }
}

// Streaming chunks read value or "" and then assert the result is a string.
pub(super) fn stream_text(block: &Object, key: &'static str) -> Result<String, ChatOutputError> {
    let Some(value) = block.get(key) else {
        return Ok(String::new());
    };
    if let Json::Text(text) = value {
        return Ok(text.clone());
    }
    if value.is_truthy() {
        return Err(ChatOutputError::NonTextThinkingField { field: key });
    }
    Ok(String::new())
}

fn tool_use_entry(
    tool_call: &ToolCallRecord,
    tool_name_mapping: Option<&IndexMap<String, String>>,
) -> Result<ContentEntry, ChatOutputError> {
    let function = tool_call.function.as_ref();
    let truncated_name = function
        .and_then(|details| details.name.as_deref())
        .unwrap_or("");
    let original_name = tool_name_mapping
        .and_then(|names| names.get(truncated_name))
        .map_or_else(|| truncated_name.to_owned(), ToOwned::to_owned);
    let arguments = function
        .and_then(|details| details.arguments.as_deref())
        .unwrap_or("");
    let input = parse_tool_call_arguments(
        arguments,
        Some(original_name.as_str()),
        Some(ARGUMENT_CONTEXT),
    )
    .map_err(|source| ChatOutputError::Arguments {
        tool_name: original_name.clone(),
        context: ARGUMENT_CONTEXT,
        arguments: arguments.to_owned(),
        source: Box::new(source),
    })?;
    if !matches!(input, Json::Object(_)) {
        return Err(ChatOutputError::NonObjectArguments {
            tool_name: original_name,
        });
    }
    let signature = tool_call.signature().filter(|value| !value.is_empty());
    Ok(ContentEntry::Block(ContentBlock::ToolUse {
        id: normalize_tool_use_id(tool_call.id.as_deref().unwrap_or("")),
        name: original_name,
        input,
        provider_specific_fields: signature.map(|signature| SignatureFields { signature }),
    }))
}

impl ToolCallRecord {
    // B1117 reads the signature through its own field records, so the two mappings are rebuilt for it.
    pub(super) fn signature(&self) -> Option<String> {
        let function_fields = self
            .function
            .as_ref()
            .map_or_else(Object::default, |details| {
                details.provider_specific_fields.clone()
            });
        let fields = ToolCallFields {
            provider_specific_fields: self.provider_specific_fields.clone(),
            function: FunctionFields {
                provider_specific_fields: function_fields,
            },
        };
        tool_call_signature(&fields).map(ToOwned::to_owned)
    }
}
