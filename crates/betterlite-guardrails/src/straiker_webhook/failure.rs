use serde::{Deserialize, Serialize};

const RETRY_STATUS: [u16; 6] = [408, 429, 500, 502, 503, 504];
const UNREACHABLE_STATUS: [u16; 3] = [502, 503, 504];
const MAX_ERROR_TEXT_CHARS: usize = 200;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct WebhookFailure {
    pub message: String,
    pub is_unreachable: bool,
    pub retryable: bool,
}

#[must_use]
pub fn status_failure(status: u16, text: &str) -> WebhookFailure {
    let detail: String = text.chars().take(MAX_ERROR_TEXT_CHARS).collect();
    WebhookFailure {
        message: format!("HTTP {status}: {detail}"),
        is_unreachable: UNREACHABLE_STATUS.contains(&status),
        retryable: RETRY_STATUS.contains(&status),
    }
}

mod tests;
