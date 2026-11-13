#![no_main]

use std::str;

use betterlite_provider::watsonx_text::reply::decode_text_generation_reply;
use betterlite_provider::watsonx_text::stream::decode_stream_line;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _reply = decode_text_generation_reply(bytes);
    if let Ok(line) = str::from_utf8(bytes) {
        let _chunk = decode_stream_line(line);
    }
});
