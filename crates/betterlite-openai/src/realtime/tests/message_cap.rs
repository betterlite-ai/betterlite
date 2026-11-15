use betterlite_config::sections::openai_realtime::DEFAULT_MAX_MESSAGE_SIZE_BYTES;

use crate::realtime::failure::RealtimeClose;
use crate::realtime::message_cap::{
    MESSAGE_TOO_BIG_CODE, MESSAGE_TOO_BIG_REASON, oversized_message_close,
};

const OVERRIDE_CAP: usize = 1024;

#[test]
fn message_cap_closes_1009() {
    assert_eq!(MESSAGE_TOO_BIG_CODE, 1009);
    assert_eq!(MESSAGE_TOO_BIG_REASON, "message too big");
    assert_eq!(DEFAULT_MAX_MESSAGE_SIZE_BYTES, 16 * 1024 * 1024);

    assert_eq!(
        oversized_message_close(
            DEFAULT_MAX_MESSAGE_SIZE_BYTES,
            DEFAULT_MAX_MESSAGE_SIZE_BYTES
        ),
        None
    );
    assert_eq!(
        oversized_message_close(
            DEFAULT_MAX_MESSAGE_SIZE_BYTES - 1,
            DEFAULT_MAX_MESSAGE_SIZE_BYTES
        ),
        None
    );
    let at_default_cap = oversized_message_close(
        DEFAULT_MAX_MESSAGE_SIZE_BYTES + 1,
        DEFAULT_MAX_MESSAGE_SIZE_BYTES,
    )
    .expect("a message past the 16 MiB default closes the socket");
    assert_eq!(
        at_default_cap,
        RealtimeClose {
            code: 1009,
            reason: String::from("message too big"),
        }
    );

    assert_eq!(oversized_message_close(OVERRIDE_CAP, OVERRIDE_CAP), None);
    let at_override_cap = oversized_message_close(OVERRIDE_CAP + 1, OVERRIDE_CAP)
        .expect("a message past the explicit override closes the socket");
    assert_eq!(at_override_cap.code, 1009);
    assert_eq!(at_override_cap.reason, "message too big");
}
