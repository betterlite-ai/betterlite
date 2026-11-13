#![no_main]

use betterlite_openai::vector_stores::reply::{ReplyKind, decode_vector_reply};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    if let Some((&tag, body)) = bytes.split_first()
        && let Some(kind) = ReplyKind::from_tag(tag)
    {
        let _decoded = decode_vector_reply(kind, body);
    }
});
