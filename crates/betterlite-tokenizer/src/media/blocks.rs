use betterlite_prompts::media::inline::RemoteMediaFetcher;

use crate::count::text::Texter;
use crate::image::tokens::Mode;

use super::count::{MediaCounter, Warning, apply_fallback, count_items, count_text};
use super::error::MediaError;
use super::image_count::{image_source_tokens, image_tokens, image_url_parts};
use super::wire::{Block, Document, FileObject, NestedContent, Object, Source};

pub(crate) async fn count_block<Fetcher>(
    counter: &MediaCounter<'_, Fetcher>,
    block: Block,
    depth: usize,
    warnings: &mut Vec<Warning>,
) -> Result<u64, MediaError>
where
    Fetcher: RemoteMediaFetcher + Sync,
{
    match block {
        Block::Text { text } => count_text(counter.texter, &text).await,
        Block::ImageUrl { image_url } => {
            let (url, detail) = image_url_parts(&image_url)?;
            image_tokens(counter, url, detail, warnings).await
        }
        Block::Image { source } => image_source_tokens(counter, &source, warnings).await,
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
            count_document(counter, document, depth, warnings).await
        }
        Block::File { file } => count_file(counter, file, warnings).await,
        Block::ToolUse {
            name,
            input,
            caller,
        } => count_tool_use(counter.texter, name, input, caller).await,
        Block::ToolResult { content, .. } => match content {
            Some(value) => count_nested(counter, value, depth, warnings).await,
            None => Ok(0),
        },
        Block::Thinking { thinking } => count_text(counter.texter, &thinking).await,
        Block::RedactedThinking => Ok(0),
        Block::ToolReference { tool_name } => count_text(counter.texter, &tool_name).await,
        Block::Unsupported => Err(MediaError::UnsupportedNestedBlock),
    }
}

async fn count_document<Fetcher>(
    counter: &MediaCounter<'_, Fetcher>,
    document: Document,
    depth: usize,
    warnings: &mut Vec<Warning>,
) -> Result<u64, MediaError>
where
    Fetcher: RemoteMediaFetcher + Sync,
{
    let Document {
        source,
        title,
        context,
    } = document;
    let mut total = 0_u64;
    for text in [title, context] {
        let tokens = count_text(counter.texter, &text).await?;
        total = total.checked_add(tokens).ok_or(MediaError::Overflow)?;
    }
    let nested = match source {
        Source::Text { data } => count_text(counter.texter, &data).await?,
        Source::Content { content } => count_nested(counter, content, depth, warnings).await?,
        Source::Url { url } => image_tokens(counter, &url, Mode::Auto, warnings).await?,
        Source::Base64 | Source::File | Source::Other => {
            image_tokens(counter, "", Mode::Auto, warnings).await?
        }
    };
    total.checked_add(nested).ok_or(MediaError::Overflow)
}

async fn count_file<Fetcher>(
    counter: &MediaCounter<'_, Fetcher>,
    file: FileObject,
    warnings: &mut Vec<Warning>,
) -> Result<u64, MediaError>
where
    Fetcher: RemoteMediaFetcher + Sync,
{
    let name_tokens = count_text(counter.texter, &file.filename).await?;
    if file.file_data.is_empty() {
        return Ok(name_tokens);
    }
    let data_tokens = image_tokens(counter, &file.file_data, Mode::Auto, warnings).await?;
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

async fn count_nested<Fetcher>(
    counter: &MediaCounter<'_, Fetcher>,
    content: NestedContent,
    depth: usize,
    warnings: &mut Vec<Warning>,
) -> Result<u64, MediaError>
where
    Fetcher: RemoteMediaFetcher + Sync,
{
    match content {
        NestedContent::Text(text) => count_text(counter.texter, &text).await,
        NestedContent::Items(items) => {
            let next = depth.saturating_add(1);
            Box::pin(count_items(counter, &items, next, warnings)).await
        }
        NestedContent::Unreadable(_) => {
            apply_fallback(MediaError::UnreadableNested, counter.settings, warnings)
        }
    }
}
