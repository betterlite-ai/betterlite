use http::HeaderMap;

use crate::credentials::ANTHROPIC_VERSION;

const ANTHROPIC_BETA_HEADER: &str = "anthropic-beta";
const ANTHROPIC_VERSION_HEADER: &str = "anthropic-version";

#[must_use]
pub fn supported_prediction_headers(headers: &HeaderMap) -> bool {
    headers.iter().all(|(name, value)| {
        !name.as_str().eq_ignore_ascii_case(ANTHROPIC_BETA_HEADER)
            && (!name.as_str().eq_ignore_ascii_case(ANTHROPIC_VERSION_HEADER)
                || value.as_bytes() == ANTHROPIC_VERSION.as_bytes())
    })
}
