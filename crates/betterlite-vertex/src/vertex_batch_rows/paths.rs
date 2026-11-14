use betterlite_context::cloud_storage::VERTEX_AI_MANAGED_GCS_PREFIX;
use percent_encoding::percent_decode_str;
use regex::Regex;

const MODEL_PATH_PATTERN: &str = "publishers/[^/]+/models/([^/?]+)";

#[must_use]
pub fn model_from_managed_gcs_url(url: &str) -> Option<String> {
    let decoded = percent_decode_str(url).decode_utf8().ok()?;
    let captures = Regex::new(MODEL_PATH_PATTERN).ok()?.captures(&decoded)?;
    Some(captures.get(1)?.as_str().to_owned())
}

// the first managed prefix decides, so a later passthrough segment does not count
#[must_use]
pub fn is_passthrough_managed_gcs_url(url: &str) -> bool {
    percent_decode_str(url).decode_utf8().is_ok_and(|decoded| {
        decoded
            .split_once(VERTEX_AI_MANAGED_GCS_PREFIX)
            .is_some_and(|(_head, tail)| tail.starts_with("passthrough/"))
    })
}
