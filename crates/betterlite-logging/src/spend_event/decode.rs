use crate::spend_event::MAX_EVENT_BYTES;
use crate::spend_event::error::SpendEventDecodeError;
use crate::spend_event::event::SpendEvent;

// Consumes one JSON line, and anything but whitespace after the event is an error
pub fn decode_spend_event(line: &[u8]) -> Result<SpendEvent, SpendEventDecodeError> {
    if line.len() > MAX_EVENT_BYTES {
        return Err(SpendEventDecodeError::TooLarge {
            length: line.len(),
            limit: MAX_EVENT_BYTES,
        });
    }
    serde_json::from_slice(line).map_err(|source| SpendEventDecodeError::Event { source })
}
