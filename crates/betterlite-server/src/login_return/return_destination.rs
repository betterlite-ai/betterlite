use std::str::{Utf8Error, from_utf8};

use thiserror::Error;

pub const MAX_RETURN_DESTINATION_BYTES: usize = 0x10_0000;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ReturnDestinationError {
    #[error("the return destination is {length} bytes, over the 1048576-byte limit")]
    TooLong { length: usize },
    #[error("the return destination is not valid UTF-8")]
    NotUtf8(#[source] Utf8Error),
}

pub fn decode_return_destination(bytes: &[u8]) -> Result<String, ReturnDestinationError> {
    if bytes.len() > MAX_RETURN_DESTINATION_BYTES {
        return Err(ReturnDestinationError::TooLong {
            length: bytes.len(),
        });
    }
    from_utf8(bytes)
        .map(str::to_owned)
        .map_err(ReturnDestinationError::NotUtf8)
}

// ui_sso.py _is_same_origin_return_path: one leading slash, no //, no backslash, no control, space or DEL.
#[must_use]
pub fn is_same_origin_return_path(return_to: &str) -> bool {
    if !return_to.starts_with('/') || return_to.starts_with("//") || return_to.contains('\\') {
        return false;
    }
    !return_to
        .chars()
        .any(|character| character.is_ascii_control() || character == ' ')
}
