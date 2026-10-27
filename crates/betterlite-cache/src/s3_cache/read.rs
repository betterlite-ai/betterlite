use std::time::SystemTime;

use aws_sdk_s3::operation::get_object::GetObjectOutput;
use aws_smithy_types::date_time::{DateTime, Format};

use super::S3Cache;
use super::body::decode_body;
use super::entry::CachedEntry;

pub(super) async fn read_body(cache: &S3Cache, mut output: GetObjectOutput) -> Option<CachedEntry> {
    if !unexpired(cache, output.expires_string()) {
        return None;
    }
    let limit = cache.max_object_bytes.get();
    if output
        .content_length()
        .is_some_and(|length| usize::try_from(length).map_or(true, |size| size > limit))
    {
        tracing::error!(
            error = "content length exceeds limit",
            "S3 cache read failed"
        );
        return None;
    }
    let mut bytes = Vec::new();
    while let Some(result) = output.body.next().await {
        let chunk = match result {
            Ok(chunk) => chunk,
            Err(source) => {
                tracing::error!(error = %source, "S3 cache body failed");
                return None;
            }
        };
        if chunk.len() > limit.saturating_sub(bytes.len()) {
            tracing::error!(error = "body exceeds limit", "S3 cache read failed");
            return None;
        }
        bytes.extend_from_slice(&chunk);
    }
    match decode_body(&bytes) {
        Ok(value) => Some(value),
        Err(source) => {
            tracing::error!(error = %source, "S3 cache decode failed");
            None
        }
    }
}

fn unexpired(cache: &S3Cache, expires: Option<&str>) -> bool {
    let Some(text) = expires else {
        return true;
    };
    match DateTime::from_str(text, Format::HttpDate) {
        Ok(timestamp) => match SystemTime::try_from(timestamp) {
            Ok(expiration) => (cache.clock)() <= expiration,
            Err(source) => {
                tracing::error!(error = %source, "S3 cache expiry invalid");
                false
            }
        },
        Err(source) => {
            tracing::error!(error = %source, "S3 cache expiry invalid");
            false
        }
    }
}
