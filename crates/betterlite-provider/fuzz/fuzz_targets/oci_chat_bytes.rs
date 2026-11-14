#![no_main]

use std::str::from_utf8;

use betterlite_provider::oci_chat::response::decode_chat_body;
use betterlite_provider::oci_chat::stream::SseDecoder;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _accepted = decode_chat_body(200, bytes);
    let _rejected = decode_chat_body(500, bytes);
    let Ok(text) = from_utf8(bytes) else {
        return;
    };
    let mut decoder = SseDecoder::default();
    let _pushed = decoder.push(text);
    let _finished = decoder.finish();
});
