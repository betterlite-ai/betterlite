use std::io::SeekFrom;
use std::num::ParseIntError;

use betterlite_context::aws_partition::get_aws_partition;
use chrono::DateTime;
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, percent_encode};
use rust_decimal::Decimal;
use symphonia_core::io::MediaSource;
use thiserror::Error;
use url::Url;

// LiteLLM's TRANSCRIBE_MAX_MEDIA_BYTES, the largest audio file Transcribe accepts
pub const MAX_MEDIA_BYTES: u64 = 2 * 1024 * 1024 * 1024;
// LiteLLM's TRANSCRIBE_MEDIA_LAST_MODIFIED_TOLERANCE_SECONDS; S3 dates carry whole seconds
pub const MODIFIED_TOLERANCE_SECONDS: f64 = 1.0;

// Python's urllib.parse.quote keeps the unreserved characters and the default safe slash
const QUOTE_SAFE: AsciiSet = NON_ALPHANUMERIC
    .remove(b'_')
    .remove(b'.')
    .remove(b'-')
    .remove(b'~')
    .remove(b'/');

#[derive(Debug, Error)]
pub enum MediaSourceError {
    #[error("the media URI is not a URL")]
    Url(#[source] url::ParseError),
    #[error("the content-length header {value:?} is not an integer")]
    ContentLength {
        value: String,
        #[source]
        source: ParseIntError,
    },
}

pub fn s3_media_url(
    media_uri: &str,
    aws_region_name: &str,
) -> Result<Option<String>, MediaSourceError> {
    let dns_suffix = get_aws_partition(Some(aws_region_name)).dns_suffix;
    if let Some(resource) = media_uri.strip_prefix("s3://") {
        let (bucket, key) = resource.split_once('/').unwrap_or((resource, ""));
        let encoded = percent_encode(key.as_bytes(), &QUOTE_SAFE);
        let url = if bucket.contains('.') {
            format!("https://s3.{aws_region_name}.{dns_suffix}/{bucket}/{encoded}")
        } else {
            format!("https://{bucket}.s3.{aws_region_name}.{dns_suffix}/{encoded}")
        };
        return Ok(Some(url));
    }
    let url = Url::parse(media_uri).map_err(MediaSourceError::Url)?;
    let own_host = format!(".{dns_suffix}");
    let accepted =
        url.scheme() == "https" && url.host_str().is_some_and(|host| host.ends_with(&own_host));
    Ok(accepted.then(|| media_uri.to_owned()))
}

#[must_use]
pub fn media_predates_job(headers: &[(String, String)], job_created_at: f64) -> bool {
    let Some((_, value)) = headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("last-modified"))
    else {
        return false;
    };
    let Ok(modified) = DateTime::parse_from_rfc2822(value) else {
        return false;
    };
    let Some(limit) = Decimal::from_f64_retain(job_created_at)
        .zip(Decimal::from_f64_retain(MODIFIED_TOLERANCE_SECONDS))
        .and_then(|(created_at, tolerance)| created_at.checked_add(tolerance))
    else {
        return false;
    };
    Decimal::from(modified.timestamp()) <= limit
}

pub fn content_length_within_limit(headers: &[(String, String)]) -> Result<bool, MediaSourceError> {
    let Some((_, declared)) = headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
    else {
        return Ok(true);
    };
    let bytes: u64 = declared
        .trim()
        .parse()
        .map_err(|source| MediaSourceError::ContentLength {
            value: declared.clone(),
            source,
        })?;
    Ok(bytes <= MAX_MEDIA_BYTES)
}

#[derive(Debug)]
pub struct MediaSizeGuard {
    received: u64,
    cap: u64,
}

impl Default for MediaSizeGuard {
    fn default() -> Self {
        Self::from(MAX_MEDIA_BYTES)
    }
}

impl From<u64> for MediaSizeGuard {
    fn from(cap: u64) -> Self {
        Self { received: 0, cap }
    }
}

impl MediaSizeGuard {
    #[must_use]
    pub fn accept(&mut self, bytes: usize) -> bool {
        // a chunk wider than u64 is already past the cap
        let Ok(byte_count) = u64::try_from(bytes) else {
            return false;
        };
        self.received = self.received.saturating_add(byte_count);
        self.received <= self.cap
    }
}

#[must_use]
pub fn media_duration_seconds<R>(mut media: R) -> Option<Decimal>
where
    R: MediaSource,
{
    let mut header = [0_u8; 12];
    if media.read_exact(&mut header).is_err() {
        tracing::warn!("Transcribe media is shorter than a container header");
        return None;
    }
    if media.seek(SeekFrom::Start(0)).is_err() {
        tracing::warn!("Transcribe media cannot be rewound for decoding");
        return None;
    }
    let Some(container) = decoders::Container::sniff(header) else {
        tracing::warn!("Transcribe media does not start with a measurable container");
        return None;
    };
    let seconds = decoders::container_seconds(container, media);
    if seconds.is_none() {
        tracing::warn!("Transcribe media could not be decoded for its duration");
    }
    seconds
}

mod decoders;
mod tests;
