use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::error::SessionStateError;
use super::records::decoded_record;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionMessage<'message> {
    Text(&'message str),
    Binary(&'message [u8]),
    Record(&'message IndexMap<String, Json>),
}

pub(crate) fn decoded_message(
    message: SessionMessage<'_>,
) -> Result<IndexMap<String, Json>, SessionStateError> {
    match message {
        SessionMessage::Record(record) => Ok(record.clone()),
        SessionMessage::Text(text) => decoded_record(text.as_bytes()),
        SessionMessage::Binary(bytes) => decoded_record(bytes),
    }
}
