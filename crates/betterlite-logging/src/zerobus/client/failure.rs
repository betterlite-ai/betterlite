use std::str;

use crate::zerobus::client::types::{RETRYABLE_INGEST_STATUS_CODES, ZerobusIngestFailure};

pub(crate) fn status_failure(what: &str, status: u16, body: &[u8]) -> ZerobusIngestFailure {
    ZerobusIngestFailure {
        detail: format!("{what} returned {status}: {}", body_text(body))
            .chars()
            .take(500)
            .collect(),
        retryable: RETRYABLE_INGEST_STATUS_CODES.contains(&status),
    }
}

// A rejected body that is not UTF-8 keeps the text up to its first invalid byte
fn body_text(body: &[u8]) -> &str {
    match str::from_utf8(body) {
        Ok(text) => text,
        Err(invalid) => str::from_utf8(body.get(..invalid.valid_up_to()).unwrap_or_default())
            .unwrap_or_default(),
    }
}
