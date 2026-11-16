use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::message_content::ContentPart;
use betterlite_prompts::message_fields::image_hoist::TOOL_RESULT_IMAGE_BOUNDARY;
use betterlite_prompts::reasoning_fields::replay_groups::reasoning_items_from_blocks;

use super::error::MessagesResponsesInputError;
use super::items::{InputItem, MessagePart};
use super::media::{document_part, image_url};
use super::records::{
    Block, Content, MessageRecord, MessageRole, RedactedThinkingBlock, ThinkingBlockRecord,
    ToolUseBlock,
};
use super::system::system_parts;
use super::tool_results::tool_result_value;

pub fn messages_to_input(
    messages: &[MessageRecord],
) -> Result<Vec<InputItem>, MessagesResponsesInputError> {
    let mut items = Vec::new();
    for message in messages {
        match message.role {
            MessageRole::System => append_system(&mut items, message.content.as_ref()),
            MessageRole::User => append_user(&mut items, message.content.as_ref()),
            MessageRole::Assistant => append_assistant(&mut items, message.content.as_ref())?,
        }
    }
    Ok(items)
}

fn append_system(items: &mut Vec<InputItem>, content: Option<&Content>) {
    let parts = system_parts(content);
    if parts.is_empty() {
        return;
    }
    items.push(InputItem::Message {
        role: MessageRole::System,
        content: parts,
    });
}

fn append_user(items: &mut Vec<InputItem>, content: Option<&Content>) {
    match content {
        None => {}
        Some(Content::Text(text)) => items.push(InputItem::Message {
            role: MessageRole::User,
            content: vec![MessagePart::Text {
                text: text.clone(),
                prompt_cache_breakpoint: None,
            }],
        }),
        Some(Content::Blocks(blocks)) => append_user_blocks(items, blocks),
    }
}

fn append_user_blocks(items: &mut Vec<InputItem>, blocks: &[Block]) {
    let (outputs, images) = tool_outputs(blocks);
    items.extend(outputs);
    if !images.is_empty() {
        items.push(InputItem::Message {
            role: MessageRole::User,
            content: boundary_parts(images),
        });
    }
    let parts = user_message_parts(blocks);
    if !parts.is_empty() {
        items.push(InputItem::Message {
            role: MessageRole::User,
            content: parts,
        });
    }
}

fn tool_outputs(blocks: &[Block]) -> (Vec<InputItem>, Vec<MessagePart>) {
    let mut outputs = Vec::new();
    let mut images = Vec::new();
    for block in blocks {
        let Block::ToolResult(tool) = block else {
            continue;
        };
        let value = tool_result_value(tool.content.as_ref());
        outputs.push(InputItem::FunctionCallOutput {
            call_id: tool.tool_use_id.clone(),
            output: value.output,
        });
        images.extend(value.images);
    }
    (outputs, images)
}

fn boundary_parts(images: Vec<MessagePart>) -> Vec<MessagePart> {
    let mut parts = Vec::with_capacity(images.len().saturating_add(1));
    parts.push(MessagePart::Text {
        text: TOOL_RESULT_IMAGE_BOUNDARY.to_owned(),
        prompt_cache_breakpoint: None,
    });
    parts.extend(images);
    parts
}

fn user_message_parts(blocks: &[Block]) -> Vec<MessagePart> {
    let mut parts = Vec::new();
    for block in blocks {
        match block {
            Block::Text(text) => parts.push(MessagePart::Text {
                text: text.text.clone(),
                prompt_cache_breakpoint: text.prompt_cache_breakpoint,
            }),
            Block::Image(image) => {
                if let Some(url) = image_url(image.source.as_ref()) {
                    parts.push(MessagePart::Image {
                        image_url: url,
                        prompt_cache_breakpoint: image.prompt_cache_breakpoint,
                    });
                }
            }
            Block::Document(document) => {
                if let Some(part) = document_part(
                    document.source.as_ref(),
                    document.title.as_deref(),
                    document.prompt_cache_breakpoint,
                ) {
                    parts.push(part);
                }
            }
            Block::ToolResult(_)
            | Block::ToolUse(_)
            | Block::Thinking(_)
            | Block::RedactedThinking(_)
            | Block::Other => {}
        }
    }
    parts
}

fn append_assistant(
    items: &mut Vec<InputItem>,
    content: Option<&Content>,
) -> Result<(), MessagesResponsesInputError> {
    match content {
        None => {}
        Some(Content::Text(text)) => items.push(InputItem::Message {
            role: MessageRole::Assistant,
            content: vec![MessagePart::OutputText { text: text.clone() }],
        }),
        Some(Content::Blocks(blocks)) => append_assistant_blocks(items, blocks)?,
    }
    Ok(())
}

fn append_assistant_blocks(
    items: &mut Vec<InputItem>,
    blocks: &[Block],
) -> Result<(), MessagesResponsesInputError> {
    let mut pending = Vec::new();
    let mut index = 0;
    while let Some(block) = blocks.get(index) {
        match block {
            Block::Thinking(thinking) => pending.push(thinking_part(thinking)),
            Block::RedactedThinking(redacted) => pending.push(redacted_part(redacted)),
            Block::ToolUse(tool) => {
                append_reasoning(items, &mut pending);
                items.push(function_call(tool)?);
            }
            Block::Text(_)
            | Block::Image(_)
            | Block::Document(_)
            | Block::ToolResult(_)
            | Block::Other => {
                append_reasoning(items, &mut pending);
            }
        }
        index = index.saturating_add(1);
    }
    append_reasoning(items, &mut pending);
    let texts = assistant_text_parts(blocks);
    if !texts.is_empty() {
        items.push(InputItem::Message {
            role: MessageRole::Assistant,
            content: texts,
        });
    }
    Ok(())
}

fn thinking_part(thinking: &ThinkingBlockRecord) -> ContentPart {
    let mut fields =
        Object::from_iter([("thinking".to_owned(), Json::Text(thinking.thinking.clone()))]);
    if let Some(signature) = &thinking.signature {
        fields.insert("signature".to_owned(), Json::Text(signature.clone()));
    }
    ContentPart {
        kind: "thinking".to_owned(),
        fields,
    }
}

fn redacted_part(redacted: &RedactedThinkingBlock) -> ContentPart {
    ContentPart {
        kind: "redacted_thinking".to_owned(),
        fields: Object::from_iter([("data".to_owned(), Json::Text(redacted.data.clone()))]),
    }
}

fn append_reasoning(items: &mut Vec<InputItem>, pending: &mut Vec<ContentPart>) {
    if pending.is_empty() {
        return;
    }
    for item in reasoning_items_from_blocks(pending.as_slice()) {
        items.push(InputItem::Reasoning {
            summary: item.summary,
            encrypted_content: item.encrypted_content,
        });
    }
    pending.clear();
}

fn function_call(tool: &ToolUseBlock) -> Result<InputItem, MessagesResponsesInputError> {
    let arguments = serde_json::to_string(&tool.input).map_err(|source| {
        MessagesResponsesInputError::ToolInput {
            tool: tool.name.clone(),
            source,
        }
    })?;
    Ok(InputItem::FunctionCall {
        call_id: tool.id.clone(),
        name: tool.name.clone(),
        arguments,
    })
}

fn assistant_text_parts(blocks: &[Block]) -> Vec<MessagePart> {
    blocks
        .iter()
        .filter_map(|block| {
            let Block::Text(text) = block else {
                return None;
            };
            Some(MessagePart::OutputText {
                text: text.text.clone(),
            })
        })
        .collect()
}
