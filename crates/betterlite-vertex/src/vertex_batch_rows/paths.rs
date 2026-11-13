use percent_encoding::percent_decode_str;
use regex::Regex;

const MANAGED_GCS_PREFIX: &str = "litellm-vertex-files/";
const PASSTHROUGH_GCS_PREFIX: &str = "litellm-vertex-files/passthrough/";
const MODEL_PATH_PATTERN: &str = "publishers/[^/]+/models/([^/?]+)";

#[must_use]
pub fn model_from_managed_gcs_url(url: &str) -> Option<String> {
    let decoded = percent_decode_str(url).decode_utf8().ok()?;
    let pattern = Regex::new(MODEL_PATH_PATTERN).ok()?;
    pattern
        .captures(&decoded)
        .and_then(|captures| captures.get(1))
        .map(|model| model.as_str().to_owned())
}

// the first managed prefix decides, so a later passthrough segment does not count
#[must_use]
pub fn is_passthrough_managed_gcs_url(url: &str) -> bool {
    let Ok(decoded) = percent_decode_str(url).decode_utf8() else {
        return false;
    };
    decoded.find(MANAGED_GCS_PREFIX).is_some_and(|start| {
        decoded
            .get(start..)
            .is_some_and(|tail| tail.starts_with(PASSTHROUGH_GCS_PREFIX))
    })
}
