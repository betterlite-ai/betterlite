use betterlite_anthropic::message_replay::tool_ids::normalize_tool_use_id;
use betterlite_anthropic::stream_state::state::IdSource;
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::gemini_tool_invokes::signatures::THOUGHT_SIGNATURE_SEPARATOR;

use crate::api::messages_chat_output::content::{anthropic_finish_reason, stream_text};
use crate::api::messages_chat_output::error::ChatOutputError;
use crate::api::messages_chat_output::output::{
    ContentBlock, ContextManagement, DeltaProjection, MessageDelta, SignatureFields, StreamFrame,
};
use crate::api::messages_chat_output::records::{
    ChoiceRecord, DeltaRecord, TerminalRecord, ToolCallRecord, UsageRecord, refusal_text,
};
use crate::api::messages_chat_output::usage::project_usage;

pub fn content_block(
    choices: &[ChoiceRecord],
    ids: &dyn IdSource,
) -> Result<ContentBlock, ChatOutputError> {
    for choice in choices {
        if let Some(tool_call) = choice.delta.tool_calls.first()
            && tool_call.function.is_some()
        {
            return Ok(tool_use_start(tool_call, ids));
        }
        if has_text_or_refusal(&choice.delta) {
            return Ok(ContentBlock::Text {
                text: String::new(),
            });
        }
        if let Some(block) = thinking_start(&choice.delta)? {
            return Ok(block);
        }
    }
    Ok(ContentBlock::Text {
        text: String::new(),
    })
}

// A first tool call with a function wins; a call whose function is absent falls through the chain.
fn tool_use_start(tool_call: &ToolCallRecord, ids: &dyn IdSource) -> ContentBlock {
    let raw_id = tool_call
        .id
        .clone()
        .filter(|id| !id.is_empty())
        .unwrap_or_else(|| ids.next_id().to_string());
    let signature = raw_id
        .split_once(THOUGHT_SIGNATURE_SEPARATOR)
        .map(|(_, signature)| signature)
        .filter(|signature| !signature.is_empty());
    let name = tool_call
        .function
        .as_ref()
        .and_then(|details| details.name.clone())
        .unwrap_or_default();
    ContentBlock::ToolUse {
        id: normalize_tool_use_id(&raw_id),
        name,
        input: Json::Object(Object::new()),
        provider_specific_fields: signature.map(|value| SignatureFields {
            signature: value.to_owned(),
        }),
    }
}

fn has_text_or_refusal(delta: &DeltaRecord) -> bool {
    if delta
        .content
        .as_deref()
        .is_some_and(|text| !text.is_empty())
    {
        return true;
    }
    refusal_text(&delta.refusal, &delta.provider_specific_fields).is_some()
}

// A present thinking_blocks attribute of any length stops the reasoning_content fallback.
fn thinking_start(delta: &DeltaRecord) -> Result<Option<ContentBlock>, ChatOutputError> {
    let Some(blocks) = &delta.thinking_blocks else {
        let thinking = ContentBlock::Thinking {
            thinking: String::new(),
            signature: Some(String::new()),
        };
        return Ok((!delta.reasoning_content.is_empty()).then_some(thinking));
    };
    let Some(block) = blocks
        .first()
        .filter(|block| block.get("type").and_then(Json::as_str) == Some("thinking"))
    else {
        return Ok(None);
    };
    Ok(Some(ContentBlock::Thinking {
        thinking: stream_text(block, "thinking")?,
        signature: Some(stream_text(block, "signature")?),
    }))
}

pub fn delta(choices: &[ChoiceRecord]) -> Result<DeltaProjection, ChatOutputError> {
    let mut text = String::new();
    let mut thinking = String::new();
    let mut signature = String::new();
    let mut partial_json: Option<String> = None;
    for choice in choices {
        if let Some(content) = choice.delta.content.as_deref() {
            text.push_str(content);
        }
        if !choice.delta.tool_calls.is_empty() {
            partial_json = Some(choice_arguments(&choice.delta));
        } else if let Some(blocks) = &choice.delta.thinking_blocks {
            thinking_blocks_text(blocks, &mut thinking, &mut signature)?;
        } else {
            thinking.push_str(&choice.delta.reasoning_content);
        }
    }
    if let Some(fragment) = partial_json {
        return Ok(DeltaProjection::InputJsonDelta {
            partial_json: fragment,
        });
    }
    if !signature.is_empty() {
        return Ok(DeltaProjection::SignatureDelta { signature });
    }
    if !thinking.is_empty() {
        return Ok(DeltaProjection::ThinkingDelta { thinking });
    }
    let refusals: String = choices
        .iter()
        .filter_map(|choice| {
            refusal_text(
                &choice.delta.refusal,
                &choice.delta.provider_specific_fields,
            )
        })
        .collect();
    Ok(DeltaProjection::TextDelta {
        text: format!("{text}{refusals}"),
    })
}

// A choice with tool calls starts its arguments from scratch instead of extending the prior choice.
fn choice_arguments(delta: &DeltaRecord) -> String {
    let mut arguments = String::new();
    for tool_call in &delta.tool_calls {
        if let Some(details) = &tool_call.function
            && let Some(fragment) = &details.arguments
        {
            arguments.push_str(fragment);
        }
    }
    arguments
}

fn thinking_blocks_text(
    blocks: &[Object],
    thinking: &mut String,
    signature: &mut String,
) -> Result<(), ChatOutputError> {
    for block in blocks {
        if block.get("type").and_then(Json::as_str) == Some("thinking") {
            thinking.push_str(&stream_text(block, "thinking")?);
            signature.push_str(&stream_text(block, "signature")?);
        }
    }
    Ok(())
}

pub fn terminal_frame(record: &TerminalRecord) -> Result<StreamFrame, ChatOutputError> {
    let Some(choice) = record.choices.first() else {
        return Err(ChatOutputError::NoChoices);
    };
    // The pinned terminal test is `is not None`, so an empty reason still closes the stream.
    let Some(finish_reason) = choice.finish_reason.as_deref() else {
        return Ok(StreamFrame::ContentBlockDelta {
            index: record.index,
            delta: delta(&record.choices)?,
        });
    };
    let usage = record
        .usage
        .as_ref()
        .or(record.hidden_usage.as_ref())
        .map_or_else(|| project_usage(&UsageRecord::default()), project_usage);
    Ok(StreamFrame::MessageDelta {
        delta: MessageDelta {
            stop_reason: anthropic_finish_reason(finish_reason),
        },
        usage,
        context_management: (!record.applied_edits.is_empty()).then(|| ContextManagement {
            applied_edits: record.applied_edits.clone(),
        }),
    })
}
