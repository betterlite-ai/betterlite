use std::borrow::Cow;
use std::sync::atomic::Ordering;

use betterlite_prompts::media::inline::RemoteMediaFetcher;
use betterlite_prompts::media::limits::MAX_REMOTE_URLS;

use crate::image::tokens::{DEFAULT_BASE_TOKENS, Mode, calculate_img_tokens};

use super::count::{MediaCounter, Warning};
use super::error::MediaError;
use super::settings::ImageCounting;
use super::wire::{ImageUrlValue, Source};

pub(crate) async fn image_source_tokens<Fetcher>(
    counter: &MediaCounter<'_, Fetcher>,
    source: &Source,
    warnings: &mut Vec<Warning>,
) -> Result<u64, MediaError>
where
    Fetcher: RemoteMediaFetcher + Sync,
{
    match source {
        Source::Url { url } => image_tokens(counter, url, Mode::Auto, warnings).await,
        Source::Base64 | Source::File | Source::Other => {
            image_tokens(counter, "", Mode::Auto, warnings).await
        }
        Source::Text { .. } => Err(MediaError::UnsupportedSource { kind: "text" }),
        Source::Content { .. } => Err(MediaError::UnsupportedSource { kind: "content" }),
    }
}

pub(crate) fn image_url_parts(value: &ImageUrlValue) -> Result<(&str, Mode), MediaError> {
    match value {
        ImageUrlValue::Text(text) if text.trim().is_empty() => Err(MediaError::BlankImageUrl),
        ImageUrlValue::Text(text) => Ok((text.as_str(), Mode::Auto)),
        ImageUrlValue::Object(object) if object.url.is_empty() => Err(MediaError::EmptyImageUrl),
        ImageUrlValue::Object(object) => Ok((object.url.as_str(), object.detail)),
    }
}

pub(crate) async fn image_tokens<Fetcher>(
    counter: &MediaCounter<'_, Fetcher>,
    data: &str,
    mode: Mode,
    warnings: &mut Vec<Warning>,
) -> Result<u64, MediaError>
where
    Fetcher: RemoteMediaFetcher + Sync,
{
    let remote = data.starts_with("http://") || data.starts_with("https://");
    let default_count = counter.settings.image_counting == ImageCounting::DefaultCount;
    if remote && default_count {
        warnings.push(Warning::RemoteImageDefaultCount);
    }
    let bytes = if remote && !default_count && mode == Mode::High {
        // check before fetching so the url past the cap never reaches the downloader
        if counter.remote_fetches.load(Ordering::SeqCst) >= MAX_REMOTE_URLS {
            return Err(MediaError::TooManyRemoteFetches {
                limit: MAX_REMOTE_URLS,
            });
        }
        counter.remote_fetches.fetch_add(1, Ordering::SeqCst);
        Cow::Owned(counter.fetcher.convert_url(data).await?)
    } else {
        Cow::Borrowed(data)
    };
    let tokens = calculate_img_tokens(
        &bytes,
        mode,
        DEFAULT_BASE_TOKENS,
        default_count,
        &counter.settings.image,
    )?;
    Ok(tokens)
}
