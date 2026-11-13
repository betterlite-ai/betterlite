use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use thiserror::Error;

// Python's urllib.parse.quote(safe="") leaves the RFC 3986 unreserved characters alone
const QUOTE_SET: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

#[derive(Debug, Error, PartialEq, Eq)]
pub enum SanitizationError {
    #[error("Invalid identifier '{identifier}': contains disallowed characters")]
    DisallowedCharacters { identifier: String },
    #[error("Invalid identifier '{identifier}': path traversal detected")]
    PathTraversal { identifier: String },
}

pub fn sanitize_id(identifier: &str) -> Result<String, SanitizationError> {
    if identifier.contains(['/', '\\', '#', '?']) {
        return Err(SanitizationError::DisallowedCharacters {
            identifier: identifier.to_owned(),
        });
    }
    if identifier.contains("..") {
        return Err(SanitizationError::PathTraversal {
            identifier: identifier.to_owned(),
        });
    }
    Ok(utf8_percent_encode(identifier, QUOTE_SET).to_string())
}

mod tests;
