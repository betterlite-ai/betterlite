use chrono::NaiveDateTime;
use sha2::{Digest as _, Sha256};

use crate::s3_transport::options::PartitionGranularity;

#[cfg(test)]
mod tests;

const MAX_OBJECT_KEY_BYTES: usize = 1024;
// S3 allows 2048 bytes of metadata headers and Content-Disposition counts against them
const MAX_DOWNLOAD_FILENAME_BYTES: usize = 1024;
const BOUNDED_HEAD_BYTES: usize = 64;
const PREFIX_DIGEST_CHARS: usize = 16;
const MIN_BOUNDED_FILE_NAME_BYTES: usize = 64;
const JSON_SUFFIX: &str = ".json";

#[must_use]
pub fn object_download_filename(start_time: NaiveDateTime, response_id: &str) -> String {
    let stamp = start_time.format("%Y-%m-%dT%H-%M-%S-%6f");
    let file_name = format!("time-{stamp}_{response_id}");
    let sanitized = format!("time-{stamp}_{}", response_id.replace(['/', '"'], "_"));
    let budget = MAX_DOWNLOAD_FILENAME_BYTES.saturating_sub(JSON_SUFFIX.len());
    if sanitized.len() <= budget {
        return format!("{sanitized}{JSON_SUFFIX}");
    }
    let bounded = bounded_file_name(&file_name, &sanitized, budget);
    format!("{bounded}{JSON_SUFFIX}")
}

#[must_use]
pub fn object_key(
    s3_path: &str,
    prefix: &str,
    start_time: NaiveDateTime,
    file_name: &str,
    granularity: PartitionGranularity,
) -> String {
    let sanitized = file_name.replace(['/', ':'], "_");
    let configured_prefix = if s3_path.is_empty() {
        prefix.to_owned()
    } else {
        format!("{}/{prefix}", s3_path.trim_end_matches('/'))
    };
    let date_segment = match granularity {
        PartitionGranularity::Day => start_time.format("%Y-%m-%d/"),
        PartitionGranularity::Hour => start_time.format("%Y-%m-%d/%H/"),
    }
    .to_string();
    let key = format!("{configured_prefix}{date_segment}{sanitized}{JSON_SUFFIX}");
    if key.len() <= MAX_OBJECT_KEY_BYTES {
        return key;
    }
    // shorten the file name first so prefix scoped IAM policies and lifecycle rules keep matching
    let budget = MAX_OBJECT_KEY_BYTES
        .saturating_sub(date_segment.len())
        .saturating_sub(JSON_SUFFIX.len());
    let prefix_bytes = configured_prefix.len();
    if prefix_bytes.saturating_add(MIN_BOUNDED_FILE_NAME_BYTES) <= budget {
        let bounded = bounded_file_name(file_name, &sanitized, budget.saturating_sub(prefix_bytes));
        return format!("{configured_prefix}{date_segment}{bounded}{JSON_SUFFIX}");
    }
    let shortest = bounded_file_name(file_name, &sanitized, MIN_BOUNDED_FILE_NAME_BYTES);
    let short_prefix = bounded_prefix(&configured_prefix, budget.saturating_sub(shortest.len()));
    format!("{short_prefix}{date_segment}{shortest}{JSON_SUFFIX}")
}

fn bounded_file_name(file_name: &str, sanitized: &str, max_bytes: usize) -> String {
    let digest = hex::encode(Sha256::digest(file_name.as_bytes()));
    let head_budget = max_bytes
        .saturating_sub(digest.len().saturating_add(1))
        .min(BOUNDED_HEAD_BYTES);
    let head = sanitized
        .get(..sanitized.floor_char_boundary(head_budget))
        .unwrap_or_default();
    if head.is_empty() {
        digest
    } else {
        format!("{head}_{digest}")
    }
}

fn bounded_prefix(configured_prefix: &str, max_bytes: usize) -> String {
    let digest = hex::encode(Sha256::digest(configured_prefix.as_bytes()));
    let digest_segment = format!("{}/", digest.get(..PREFIX_DIGEST_CHARS).unwrap_or_default());
    if max_bytes < digest_segment.len() {
        return String::new();
    }
    let head_budget = max_bytes
        .saturating_sub(digest_segment.len())
        .saturating_sub(1);
    let head = configured_prefix
        .get(..configured_prefix.floor_char_boundary(head_budget))
        .unwrap_or_default()
        .trim_end_matches('/');
    if head.is_empty() {
        digest_segment
    } else {
        format!("{head}/{digest_segment}")
    }
}
