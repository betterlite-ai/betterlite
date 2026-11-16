use std::borrow::Cow;

use betterlite_prompts::media::inline::RemoteMediaFetcher;

use crate::image::tokens::{DEFAULT_BASE_TOKENS, Mode, calculate_img_tokens};

use super::count::{MediaCounter, Warning};
use super::error::MediaError;
use super::settings::ImageCounting;
use super::wire::Source;

pub(crate) async fn image_source_tokens<Fetcher: RemoteMediaFetcher + Sync>(
    counter: &MediaCounter<'_, Fetcher>,
    source: &Source,
    warnings: &mut Vec<Warning>,
) -> Result<u64, MediaError> {
    match source {
        Source::Url { url } => image_tokens(counter, url, Mode::Auto, warnings).await,
        Source::Base64 | Source::File | Source::Other => {
            image_tokens(counter, "", Mode::Auto, warnings).await
        }
        Source::Text { .. } => Err(MediaError::UnsupportedSource { kind: "text" }),
        Source::Content { .. } => Err(MediaError::UnsupportedSource { kind: "content" }),
    }
}

pub(crate) async fn image_tokens<Fetcher: RemoteMediaFetcher + Sync>(
    counter: &MediaCounter<'_, Fetcher>,
    data: &str,
    mode: Mode,
    warnings: &mut Vec<Warning>,
) -> Result<u64, MediaError> {
    let remote = data.starts_with("http://") || data.starts_with("https://");
    let default_count = counter.settings.image_counting == ImageCounting::DefaultCount;
    if remote && default_count {
        warnings.push(Warning::RemoteImageDefaultCount);
    }
    let bytes = if remote && !default_count && mode == Mode::High {
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
