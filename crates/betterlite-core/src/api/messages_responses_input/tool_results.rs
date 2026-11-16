use betterlite_prompts::message_fields::image_hoist::TOOL_RESULT_IMAGE_PLACEHOLDER;

use super::items::{MessagePart, ToolOutput};
use super::media::{document_part, image_url};
use super::records::{Block, KnownBlock, ToolResultContent};

pub(super) struct ToolResultValue {
    pub(super) output: ToolOutput,
    pub(super) images: Vec<MessagePart>,
}

pub(super) fn tool_result_value(content: Option<&ToolResultContent>) -> ToolResultValue {
    match content {
        None => text_value(""),
        Some(ToolResultContent::Text(text)) => text_value(text),
        Some(ToolResultContent::Blocks(blocks)) => block_value(blocks),
    }
}

fn text_value(text: &str) -> ToolResultValue {
    ToolResultValue {
        output: ToolOutput::Text(text.to_owned()),
        images: Vec::new(),
    }
}

fn tool_text(blocks: &[Block], has_images: bool) -> String {
    let texts: Vec<&str> = blocks
        .iter()
        .filter_map(|block| match block {
            Block::Known(KnownBlock::Text(text)) => Some(text.text.as_str()),
            Block::Known(
                KnownBlock::Image(_)
                | KnownBlock::Document(_)
                | KnownBlock::ToolResult(_)
                | KnownBlock::ToolUse(_)
                | KnownBlock::Thinking(_)
                | KnownBlock::RedactedThinking(_)
                | KnownBlock::Other,
            )
            | Block::Untyped(_) => None,
        })
        .collect();
    let joined = texts.join("\n");
    if !has_images {
        return joined;
    }
    if joined.is_empty() {
        return TOOL_RESULT_IMAGE_PLACEHOLDER.to_owned();
    }
    let mut text = joined;
    text.push('\n');
    text.push_str(TOOL_RESULT_IMAGE_PLACEHOLDER);
    text
}

fn output_value(text: &str, files: Vec<MessagePart>) -> ToolOutput {
    if files.is_empty() {
        return ToolOutput::Text(text.to_owned());
    }
    let mut parts = Vec::with_capacity(files.len().saturating_add(1));
    if !text.is_empty() {
        parts.push(MessagePart::Text {
            text: text.to_owned(),
            prompt_cache_breakpoint: None,
        });
    }
    parts.extend(files);
    ToolOutput::Parts(parts)
}

fn block_value(blocks: &[Block]) -> ToolResultValue {
    let mut files = Vec::new();
    let mut images = Vec::new();
    for block in blocks {
        match block {
            Block::Known(KnownBlock::Document(document)) => {
                if let Some(part) =
                    document_part(document.source.as_ref(), document.title.as_deref(), None)
                {
                    files.push(part);
                }
            }
            Block::Known(KnownBlock::Image(image)) => {
                if let Some(url) = image_url(image.source.as_ref()) {
                    images.push(MessagePart::Image {
                        image_url: url,
                        prompt_cache_breakpoint: None,
                    });
                }
            }
            Block::Known(
                KnownBlock::Text(_)
                | KnownBlock::ToolResult(_)
                | KnownBlock::ToolUse(_)
                | KnownBlock::Thinking(_)
                | KnownBlock::RedactedThinking(_)
                | KnownBlock::Other,
            )
            | Block::Untyped(_) => {}
        }
    }
    let text = tool_text(blocks, !images.is_empty());
    ToolResultValue {
        output: output_value(&text, files),
        images,
    }
}
