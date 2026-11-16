use betterlite_prompts::media::inline::RemoteMediaFetcher;
use serde_json::value::RawValue;

use crate::count::text::Texter;

use super::blocks::count_block;
use super::error::MediaError;
use super::settings::Settings;
use super::wire::{Block, BlockKind, ContentItem, TokenMedia};

const ROOT_DEPTH: usize = 1;
pub const MAX_CONTENT_DEPTH: usize = 32;

// clippy's five-argument limit means the values every content node shares travel as one reference
pub(crate) struct MediaCounter<'call, Fetcher> {
    pub(crate) texter: &'call Texter<'call>,
    pub(crate) settings: &'call Settings,
    pub(crate) fetcher: &'call Fetcher,
}

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

pub async fn count_token_media<Fetcher>(
    texter: &Texter<'_>,
    media: &TokenMedia,
    settings: &Settings,
    fetcher: &Fetcher,
) -> Result<Counted, MediaError>
where
    Fetcher: RemoteMediaFetcher + Sync,
{
    let mut warnings = Vec::new();
    let counter = MediaCounter {
        texter,
        settings,
        fetcher,
    };
    let parsed: Result<Vec<ContentItem>, MediaError> = media
        .content
        .iter()
        .map(|raw| parse_root_item(raw))
        .collect();
    let tokens = match parsed {
        Ok(items) => count_items(&counter, &items, ROOT_DEPTH, &mut warnings).await?,
        Err(error) => apply_fallback(error, settings, &mut warnings)?,
    };
    Ok(Counted { tokens, warnings })
}

fn parse_root_item(raw: &RawValue) -> Result<ContentItem, MediaError> {
    let parsed: ContentItem =
        serde_json::from_str(raw.get()).map_err(|source| MediaError::Json { source })?;
    if matches!(parsed, ContentItem::Block(Block::Unsupported)) {
        let named: BlockKind =
            serde_json::from_str(raw.get()).map_err(|source| MediaError::Json { source })?;
        return Err(MediaError::UnsupportedBlock { kind: named.kind });
    }
    Ok(parsed)
}

pub(crate) async fn count_items<Fetcher>(
    counter: &MediaCounter<'_, Fetcher>,
    items: &[ContentItem],
    depth: usize,
    warnings: &mut Vec<Warning>,
) -> Result<u64, MediaError>
where
    Fetcher: RemoteMediaFetcher + Sync,
{
    if depth > MAX_CONTENT_DEPTH {
        return Err(MediaError::TooDeep {
            limit: MAX_CONTENT_DEPTH,
        });
    }
    let outcome: Result<u64, MediaError> = async {
        let mut total = 0_u64;
        for item in items {
            let tokens = match item {
                ContentItem::Text(text) => count_text(counter.texter, text).await?,
                ContentItem::Block(block) => {
                    count_block(counter, block.clone(), depth, warnings).await?
                }
            };
            total = total.checked_add(tokens).ok_or(MediaError::Overflow)?;
        }
        Ok(total)
    }
    .await;
    match outcome {
        Ok(tokens) => Ok(tokens),
        Err(error) => apply_fallback(error, counter.settings, warnings),
    }
}

pub(crate) async fn count_text(texter: &Texter<'_>, text: &str) -> Result<u64, MediaError> {
    match texter.count(text).await {
        Ok(tokens) => u64::try_from(tokens).map_err(|_conversion| MediaError::Overflow),
        Err(error) => Err(MediaError::Count(error)),
    }
}

pub(crate) fn apply_fallback(
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
