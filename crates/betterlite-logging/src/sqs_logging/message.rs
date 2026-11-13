use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};

use crate::fields::payload::StandardLoggingPayload;

use super::encryption::{Encryption, EncryptionError};

pub const SQS_SEND_MESSAGE_ACTION: &str = "SendMessage";
pub const SQS_API_VERSION: &str = "2012-11-05";
pub const FORM_CONTENT_TYPE: &str = "application/x-www-form-urlencoded";

// Python's urllib.parse.quote(body, safe="") leaves only the RFC 3986 unreserved characters alone
const QUOTE_SET: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

#[derive(Debug, thiserror::Error)]
pub enum MessageError {
    #[error("the SQS message body could not be encoded as JSON")]
    Encode(#[source] serde_json::Error),
    #[error(transparent)]
    Encryption(#[from] EncryptionError),
}

#[must_use]
pub fn form_body(message: &str) -> String {
    format!(
        "Action={SQS_SEND_MESSAGE_ACTION}&Version={SQS_API_VERSION}&MessageBody={}",
        utf8_percent_encode(message, QUOTE_SET)
    )
}

pub fn message_json(
    payload: &StandardLoggingPayload,
    encryption: Option<&Encryption>,
) -> Result<String, MessageError> {
    match encryption {
        Some(encryptor) => Ok(encryptor.wrap(payload)?),
        None => serde_json::to_string(payload).map_err(MessageError::Encode),
    }
}

mod tests;
