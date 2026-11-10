use crate::count::text::Texter;
use crate::image::tokens::Mode;

use super::count::{Warning, apply_fallback, count_items, count_text};
use super::error::MediaError;
use super::image_count::{image_source_tokens, image_tokens};
use super::settings::Settings;
use super::wire::{Block, Document, FileObject, ImageUrlValue, NestedContent, Object, Source};

pub(crate) async fn count_block(
    texter: &Texter<'_>,
    block: Block,
    settings: &Settings,
    depth: usize,
    warnings: &mut Vec<Warning>,
) -> Result<u64, MediaError> {
    match block {
        Block::Text { text } => count_text(texter, &text).await,
        Block::ImageUrl { image_url } => image_url_tokens(&image_url, settings, warnings),
        Block::Image { source } => image_source_tokens(&source, settings, warnings),
        Block::Document {
            source,
            title,
            context,
        } => {
            let document = Document {
                source,
                title,
                context,
            };
            count_document(texter, document, settings, depth, warnings).await
        }
        Block::File { file } => count_file(texter, file, settings, warnings).await,
        Block::ToolUse {
            name,
            input,
            caller,
        } => count_tool_use(texter, name, input, caller).await,
        Block::ToolResult { content, .. } => match content {
            Some(value) => count_nested(texter, value, settings, depth, warnings).await,
            None => Ok(0),
        },
        Block::Thinking { thinking } => count_text(texter, &thinking).await,
        Block::RedactedThinking => Ok(0),
        Block::ToolReference { tool_name } => count_text(texter, &tool_name).await,
        Block::Unsupported => Err(MediaError::UnsupportedNestedBlock),
    }
}

fn image_url_tokens(
    value: &ImageUrlValue,
    settings: &Settings,
    warnings: &mut Vec<Warning>,
) -> Result<u64, MediaError> {
    match value {
        ImageUrlValue::Text(text) if text.trim().is_empty() => Err(MediaError::BlankImageUrl),
        ImageUrlValue::Text(text) => image_tokens(text, Mode::Auto, settings, warnings),
        ImageUrlValue::Object(object) => {
            if object.url.is_empty() {
                return Err(MediaError::EmptyImageUrl);
            }
            image_tokens(&object.url, object.detail, settings, warnings)
        }
    }
}

async fn count_document(
    texter: &Texter<'_>,
    document: Document,
    settings: &Settings,
    depth: usize,
    warnings: &mut Vec<Warning>,
) -> Result<u64, MediaError> {
    let Document {
        source,
        title,
        context,
    } = document;
    let mut total = 0_u64;
    for text in [title, context] {
        let tokens = count_text(texter, &text).await?;
        total = total.checked_add(tokens).ok_or(MediaError::Overflow)?;
    }
    let nested = match source {
        Source::Text { data } => count_text(texter, &data).await?,
        Source::Content { content } => {
            count_nested(texter, content, settings, depth, warnings).await?
        }
        Source::Url { url } => image_tokens(&url, Mode::Auto, settings, warnings)?,
        Source::Base64 | Source::File | Source::Other => {
            image_tokens("", Mode::Auto, settings, warnings)?
        }
    };
    total.checked_add(nested).ok_or(MediaError::Overflow)
}

async fn count_file(
    texter: &Texter<'_>,
    file: FileObject,
    settings: &Settings,
    warnings: &mut Vec<Warning>,
) -> Result<u64, MediaError> {
    let name_tokens = count_text(texter, &file.filename).await?;
    if file.file_data.is_empty() {
        return Ok(name_tokens);
    }
    let data_tokens = image_tokens(&file.file_data, Mode::Auto, settings, warnings)?;
    name_tokens
        .checked_add(data_tokens)
        .ok_or(MediaError::Overflow)
}

async fn count_tool_use(
    texter: &Texter<'_>,
    name: String,
    input: Option<Object>,
    caller: Option<Object>,
) -> Result<u64, MediaError> {
    let mut total = count_text(texter, &name).await?;
    for object in [input, caller].into_iter().flatten() {
        // a dict field is counted as standard JSON: docs/compatibility.md drops Python's repr text
        let text = serde_json::to_string(&object).map_err(|source| MediaError::Json { source })?;
        let tokens = count_text(texter, &text).await?;
        total = total.checked_add(tokens).ok_or(MediaError::Overflow)?;
    }
    Ok(total)
}

async fn count_nested(
    texter: &Texter<'_>,
    content: NestedContent,
    settings: &Settings,
    depth: usize,
    warnings: &mut Vec<Warning>,
) -> Result<u64, MediaError> {
    match content {
        NestedContent::Text(text) => count_text(texter, &text).await,
        NestedContent::Items(items) => {
            let next = depth.saturating_add(1);
            Box::pin(count_items(texter, &items, settings, next, warnings)).await
        }
        NestedContent::Unreadable(_) => {
            apply_fallback(MediaError::UnreadableNested, settings, warnings)
        }
    }
}
