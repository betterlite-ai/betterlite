use crate::count::text::Texter;
use crate::image::tokens::{DEFAULT_BASE_TOKENS, Mode, calculate_img_tokens};

use super::error::MediaError;
use super::settings::{ImageCounting, Settings};
use super::wire::{
    Block, ContentItem, Document, DocumentContent, DocumentSource, ImageSource, Object, RawBlock,
    TokenMedia, ToolContent,
};

const ROOT_DEPTH: usize = 1;
pub const MAX_CONTENT_DEPTH: usize = 32;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Counted {
    pub tokens: u64,
    pub warnings: Vec<Warning>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Warning {
    // the request path chose the default count for a remote image, so no fetch was needed
    RemoteImageDefaultCount,
    // LiteLLM's default_token_count replaced a content list that could not be counted
    FallbackTokenCount { tokens: u64 },
}

pub async fn count_token_media(
    texter: &Texter<'_>,
    media: &TokenMedia,
    settings: &Settings,
) -> Result<Counted, MediaError> {
    let mut warnings = Vec::new();
    let tokens = count_list(texter, &media.content, settings, ROOT_DEPTH, &mut warnings).await?;
    Ok(Counted { tokens, warnings })
}

async fn count_list(
    texter: &Texter<'_>,
    items: &[ContentItem],
    settings: &Settings,
    depth: usize,
    warnings: &mut Vec<Warning>,
) -> Result<u64, MediaError> {
    if depth > MAX_CONTENT_DEPTH {
        return Err(MediaError::TooDeep {
            limit: MAX_CONTENT_DEPTH,
        });
    }
    match count_items(texter, items, settings, depth, warnings).await {
        Ok(tokens) => Ok(tokens),
        Err(error) => apply_fallback(error, settings, warnings),
    }
}

fn apply_fallback(
    error: MediaError,
    settings: &Settings,
    warnings: &mut Vec<Warning>,
) -> Result<u64, MediaError> {
    settings.fallback_token_count.map_or_else(
        || Err(error),
        |tokens| {
            warnings.push(Warning::FallbackTokenCount { tokens });
            Ok(tokens)
        },
    )
}

async fn count_items(
    texter: &Texter<'_>,
    items: &[ContentItem],
    settings: &Settings,
    depth: usize,
    warnings: &mut Vec<Warning>,
) -> Result<u64, MediaError> {
    let mut total = 0_u64;
    for item in items {
        let tokens = match item {
            ContentItem::Text(text) => count_text(texter, text).await?,
            ContentItem::Block(raw) => count_block(texter, raw, settings, depth, warnings).await?,
        };
        total = total.checked_add(tokens).ok_or(MediaError::Overflow)?;
    }
    Ok(total)
}

async fn count_block(
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
        Block::ToolResult { content } => {
            count_tool_result(texter, content, settings, depth, warnings).await
        }
        Block::Thinking { text } => count_optional_text(texter, text.as_deref()).await,
        Block::ToolReference { tool_name } => {
            count_optional_text(texter, tool_name.as_deref()).await
        }
        Block::Unsupported { kind } => Err(MediaError::UnsupportedBlock { kind }),
    }
}

async fn count_text(texter: &Texter<'_>, text: &str) -> Result<u64, MediaError> {
    match texter.count(text).await {
        Ok(tokens) => u64::try_from(tokens).map_err(|_conversion| MediaError::Overflow),
        Err(error) => Err(MediaError::Count(error)),
    }
}

async fn count_optional_text(texter: &Texter<'_>, value: Option<&str>) -> Result<u64, MediaError> {
    match value {
        Some(text) => count_text(texter, text).await,
        None => Ok(0),
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
        DocumentSource::Text { data } => count_text(texter, &data).await?,
        DocumentSource::Content(DocumentContent::Text(text)) => count_text(texter, &text).await?,
        DocumentSource::Content(DocumentContent::Items(items)) => {
            let next = depth.saturating_add(1);
            Box::pin(count_list(texter, &items, settings, next, warnings)).await?
        }
        DocumentSource::Url { url } => image_tokens(
            url.as_deref().unwrap_or_default(),
            Mode::Auto,
            settings,
            warnings,
        )?,
        DocumentSource::Local => image_tokens("", Mode::Auto, settings, warnings)?,
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

async fn count_tool_result(
    texter: &Texter<'_>,
    content: Option<ToolContent>,
    settings: &Settings,
    depth: usize,
    warnings: &mut Vec<Warning>,
) -> Result<u64, MediaError> {
    match content {
        None => Ok(0),
        Some(ToolContent::Text(text)) => count_text(texter, &text).await,
        Some(ToolContent::Items(items)) => {
            let next = depth.saturating_add(1);
            Box::pin(count_list(texter, &items, settings, next, warnings)).await
        }
    }
}

fn image_source_tokens(
    source: &ImageSource,
    settings: &Settings,
    warnings: &mut Vec<Warning>,
) -> Result<u64, MediaError> {
    match source {
        ImageSource::Url { url } => image_tokens(
            url.as_deref().unwrap_or_default(),
            Mode::Auto,
            settings,
            warnings,
        ),
        ImageSource::Local => image_tokens("", Mode::Auto, settings, warnings),
    }
}

fn image_tokens(
    data: &str,
    mode: Mode,
    settings: &Settings,
    warnings: &mut Vec<Warning>,
) -> Result<u64, MediaError> {
    let remote = data.starts_with("http://") || data.starts_with("https://");
    let default_count = settings.image_counting == ImageCounting::DefaultCount;
    if remote && default_count {
        warnings.push(Warning::RemoteImageDefaultCount);
    }
    if remote && !default_count && mode == Mode::High {
        return Err(MediaError::RemoteNeedsDefaultCount);
    }
    let tokens = calculate_img_tokens(
        data,
        mode,
        DEFAULT_BASE_TOKENS,
        default_count,
        &settings.image,
    )?;
    Ok(tokens)
}
