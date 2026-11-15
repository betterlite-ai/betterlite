use super::failure::RealtimeClose;

pub const MESSAGE_TOO_BIG_CODE: u16 = 1009;
pub const MESSAGE_TOO_BIG_REASON: &str = "message too big";

#[must_use]
pub fn oversized_message_close(
    message_bytes: usize,
    max_size_bytes: usize,
) -> Option<RealtimeClose> {
    if message_bytes <= max_size_bytes {
        return None;
    }
    Some(RealtimeClose {
        code: MESSAGE_TOO_BIG_CODE,
        reason: MESSAGE_TOO_BIG_REASON.to_owned(),
    })
}
