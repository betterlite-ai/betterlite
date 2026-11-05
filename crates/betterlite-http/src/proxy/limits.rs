use std::time::Duration;

use reqwest::Response;
use reqwest::header::HeaderMap;
use thiserror::Error;

#[derive(Debug, Clone, Copy)]
pub struct TransportLimits {
    pub connect_timeout: Duration,
    pub read_timeout: Duration,
    pub body_bytes: usize,
    pub header_bytes: usize,
}

impl TransportLimits {
    // body_bytes is 16 MiB and header_bytes 64 KiB; the read timeout matches LiteLLM's default request timeout, so a deployment deadline is what ends a slow answer
    pub const DEFAULT: Self = Self {
        connect_timeout: Duration::from_secs(10),
        read_timeout: Duration::from_mins(100),
        body_bytes: 0x0100_0000,
        header_bytes: 0x0001_0000,
    };
}

// hyper gives up on a response head once it has buffered this many bytes (8192 + 4096 * 100) without finding its end, before header_bytes is looked at
pub const HTTP1_PARSER_CEILING: usize = 8192 + 4096 * 100;

#[derive(Debug, Error)]
#[error("the response headers are larger than {limit} bytes")]
pub struct HeadersTooLarge {
    pub limit: usize,
}

#[derive(Debug, Error)]
pub enum ReadError {
    #[error("the response body is larger than {limit} bytes")]
    TooLarge { limit: usize },
    #[error("the response body could not be read")]
    Failed(#[source] reqwest::Error),
}

pub fn check_headers(headers: &HeaderMap, limit: usize) -> Result<(), HeadersTooLarge> {
    let mut total = 0_usize;
    for (name, value) in headers {
        total = total
            .checked_add(name.as_str().len())
            .and_then(|sum| sum.checked_add(value.as_bytes().len()))
            .filter(|sum| *sum <= limit)
            .ok_or(HeadersTooLarge { limit })?;
    }
    Ok(())
}

pub async fn read_bounded(response: &mut Response, limit: usize) -> Result<Vec<u8>, ReadError> {
    let declared_too_large = response
        .content_length()
        .is_some_and(|declared| usize::try_from(declared).map_or(true, |bytes| bytes > limit));
    if declared_too_large {
        return Err(ReadError::TooLarge { limit });
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(ReadError::Failed)? {
        if body
            .len()
            .checked_add(chunk.len())
            .is_none_or(|total| total > limit)
        {
            return Err(ReadError::TooLarge { limit });
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}
