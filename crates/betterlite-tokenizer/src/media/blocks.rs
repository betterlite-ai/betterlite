use crate::count::text::Texter;
use crate::image::tokens::Mode;

use super::count::{Warning, count_list, count_optional_text, count_text};
use super::error::MediaError;
use super::image_count::{image_source_tokens, image_tokens};
use super::settings::Settings;
use super::wire::{Block, Document, NestedContent, Object, RawBlock, Source};

pub(crate) async fn count_block(
    texter: &Texter<'_>,
    raw: &RawBlock,
    settings: &Settings,
    depth: usize,
    warnings: &mut Vec<Warning>,
) -> Result<u64, MediaError> {
    match raw.parse()? {
        Block::Text { text } => count_text(texter, &text).await,
        Block::ImageUrl { image } => image_tokens(&image.url, image.detail, settings, warnings),
        Block::Image { source } => image_source_tokens(&source, settings, warnings),
        Block::Document(document) => {
            count_document(texter, document, settings, depth, warnings).await
        }
        Block::File {
            filename,
            file_data,
        } => count_file(texter, filename, file_data, settings, warnings).await,
        Block::ToolUse {
            name,
            input,
            caller,
        } => count_tool_use(texter, name, input, caller).await,
        Block::ToolResult { content } => match content {
            Some(value) => count_nested(texter, value, settings, depth, warnings).await,
            None => Ok(0),
        },
        Block::Thinking { text } => count_optional_text(texter, text.as_deref()).await,
        Block::ToolReference { tool_name } => {
            count_optional_text(texter, tool_name.as_deref()).await
        }
        Block::Unsupported { kind } => Err(MediaError::UnsupportedBlock { kind }),
    }
}

pub(crate) async fn count_nested(
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
            Box::pin(count_list(texter, &items, settings, next, warnings)).await
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
    let mut total = 0_u64;
    for text in [document.title, document.context].into_iter().flatten() {
        if !text.is_empty() {
            let tokens = count_text(texter, &text).await?;
            total = total.checked_add(tokens).ok_or(MediaError::Overflow)?;
        }
    }
    let nested = match document.source {
        Source::Text { data } => count_text(texter, &data).await?,
        Source::Content(content) => {
            count_nested(texter, content, settings, depth, warnings).await?
        }
        Source::Url { url } => image_tokens(
            url.as_deref().unwrap_or_default(),
            Mode::Auto,
            settings,
            warnings,
        )?,
        Source::Base64 | Source::File => image_tokens("", Mode::Auto, settings, warnings)?,
    };
    total.checked_add(nested).ok_or(MediaError::Overflow)
}

async fn count_file(
    texter: &Texter<'_>,
    filename: Option<String>,
    file_data: Option<String>,
    settings: &Settings,
    warnings: &mut Vec<Warning>,
) -> Result<u64, MediaError> {
    let mut total = match filename.filter(|name| !name.is_empty()) {
        Some(name) => count_text(texter, &name).await?,
        None => 0,
    };
    if let Some(data) = file_data.filter(|data| !data.is_empty()) {
        let tokens = image_tokens(&data, Mode::Auto, settings, warnings)?;
        total = total.checked_add(tokens).ok_or(MediaError::Overflow)?;
    }
    Ok(total)
}

async fn count_tool_use(
    texter: &Texter<'_>,
    name: Option<String>,
    input: Option<&Object>,
    caller: Option<&Object>,
) -> Result<u64, MediaError> {
    let mut total = count_optional_text(texter, name.as_deref()).await?;
    for object in [input, caller].into_iter().flatten() {
        // a dict field is counted as standard JSON: docs/compatibility.md drops Python's repr text
        let text = serde_json::to_string(object).map_err(|source| MediaError::Json { source })?;
        let tokens = count_text(texter, &text).await?;
        total = total.checked_add(tokens).ok_or(MediaError::Overflow)?;
    }
    Ok(total)
}
