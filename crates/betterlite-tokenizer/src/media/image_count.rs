use crate::image::tokens::{DEFAULT_BASE_TOKENS, Mode, calculate_img_tokens};

use super::count::Warning;
use super::error::MediaError;
use super::settings::{ImageCounting, Settings};
use super::wire::Source;

pub(crate) fn image_source_tokens(
    source: &Source,
    settings: &Settings,
    warnings: &mut Vec<Warning>,
) -> Result<u64, MediaError> {
    match source {
        Source::Url { url } => image_tokens(url, Mode::Auto, settings, warnings),
        Source::Base64 | Source::File | Source::Other => {
            image_tokens("", Mode::Auto, settings, warnings)
        }
        Source::Text { .. } => Err(MediaError::UnsupportedSource { kind: "text" }),
        Source::Content { .. } => Err(MediaError::UnsupportedSource { kind: "content" }),
    }
}

pub(crate) fn image_tokens(
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
