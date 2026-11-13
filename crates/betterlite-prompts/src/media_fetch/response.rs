use std::ops::{Add as _, Div as _, Mul as _};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use betterlite_http::error::HttpError;
use futures_util::StreamExt as _;
use futures_util::stream::BoxStream;
use http::HeaderMap;
use http::header::{CONTENT_LENGTH, CONTENT_TYPE};
use thiserror::Error;

use crate::media_metadata::url_mime::infer_content_type;

use super::cache::ImageCache;
use super::error::ImageFetchError;
use super::float_text::float_text;

const MEBIBYTE: f64 = 1_048_576.0;
const READ_CHUNK_BYTES: u32 = 8192;

pub struct ImageResponse {
    pub status: u16,
    pub headers: HeaderMap,
    pub body: BoxStream<'static, Result<Vec<u8>, HttpError>>,
}

#[derive(Debug, Error)]
pub enum BodyReadError {
    #[error(transparent)]
    TooLarge(#[from] ImageFetchError),
    #[error("the image size limit of {max_download_size_mb}MB cannot be counted in bytes")]
    UncountableLimit { max_download_size_mb: f64 },
    #[error("the image body could not be read")]
    Read(#[source] HttpError),
    #[error("the image body is larger than this platform can count")]
    UncountableBody,
}

#[derive(Debug, Error)]
pub enum ProcessError {
    #[error(transparent)]
    Image(#[from] ImageFetchError),
    #[error("the Content-Length header is not an integer")]
    ContentLength,
    #[error(transparent)]
    Body(#[from] BodyReadError),
}

// _process_image_response: the status and declared-size checks, the counted body, the media type and the cache
pub async fn process_image_response(
    response: ImageResponse,
    url: &str,
    max_download_size_mb: f64,
    cache: &ImageCache,
) -> Result<String, ProcessError> {
    if response.status != 200 {
        return Err(ProcessError::Image(ImageFetchError::Status {
            status: response.status,
            url: url.to_owned(),
        }));
    }
    if let Some(value) = response.headers.get(CONTENT_LENGTH) {
        let size_mb = value
            .to_str()
            .ok()
            .and_then(integer_megabytes)
            .filter(|size_mb| size_mb.is_finite())
            .ok_or(ProcessError::ContentLength)?;
        if size_mb > max_download_size_mb {
            return Err(ProcessError::Image(ImageFetchError::TooLarge {
                size: format!("{size_mb:.2}"),
                limit: float_text(max_download_size_mb),
                url: url.to_owned(),
            }));
        }
    }
    let content_type = response
        .headers
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let bytes = read_limited_body(response.body, url, max_download_size_mb).await?;
    let media_type =
        infer_content_type(url, &bytes, content_type.as_deref()).map_err(|source| {
            ProcessError::Image(ImageFetchError::UnknownMediaType {
                url: url.to_owned(),
                source,
            })
        })?;
    let data_url = format!("data:{media_type};base64,{}", BASE64.encode(&bytes));
    cache.store(url, &data_url);
    Ok(data_url)
}

// Python's int() on the header text, then int / (1024*1024) as a float
fn integer_megabytes(text: &str) -> Option<f64> {
    let trimmed = text.trim();
    let digits = trimmed.strip_prefix(['+', '-']).unwrap_or(trimmed);
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    trimmed.parse::<f64>().ok().map(|value| value.div(MEBIBYTE))
}

// The body count of iter_bytes(chunk_size=8192): a full 8192-byte chunk is counted as it arrives, the last partial one at the end
pub async fn read_limited_body(
    mut body: BoxStream<'static, Result<Vec<u8>, HttpError>>,
    url: &str,
    max_download_size_mb: f64,
) -> Result<Vec<u8>, BodyReadError> {
    let limit_bytes = max_download_size_mb.mul(MEBIBYTE);
    if !limit_bytes.is_finite() {
        return Err(BodyReadError::UncountableLimit {
            max_download_size_mb,
        });
    }
    let step = f64::from(READ_CHUNK_BYTES);
    // The first 8192-byte chunk past the limit; a limit of zero or less makes that the first chunk read
    let first_past = limit_bytes.div(step).floor().mul_add(step, step).max(step);
    let mut bytes = Vec::new();
    let mut received = 0.0_f64;
    while let Some(chunk) = body.next().await {
        let part = chunk.map_err(BodyReadError::Read)?;
        let length = u32::try_from(part.len()).map_err(|_error| BodyReadError::UncountableBody)?;
        received = received.add(f64::from(length));
        if received >= first_past {
            return Err(too_large(first_past, max_download_size_mb, url));
        }
        bytes.extend_from_slice(&part);
    }
    // Only a chunk the loop emitted is checked, so an empty body never fails a limit of zero or less
    if received > 0.0_f64 && received > limit_bytes {
        return Err(too_large(received, max_download_size_mb, url));
    }
    Ok(bytes)
}

fn too_large(size_bytes: f64, max_download_size_mb: f64, url: &str) -> BodyReadError {
    BodyReadError::TooLarge(ImageFetchError::TooLarge {
        size: format!("{:.2}", size_bytes.div(MEBIBYTE)),
        limit: float_text(max_download_size_mb),
        url: url.to_owned(),
    })
}
